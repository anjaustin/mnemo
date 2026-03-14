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

use mnemo_core::models::api_key::CallerContext;
use mnemo_core::models::edge::{Edge, ExtractedRelationship};
use mnemo_core::models::entity::{Entity, EntityType, ExtractedEntity};
use mnemo_core::traits::fulltext::FullTextStore;
use mnemo_core::traits::llm::EmbeddingConfig;
use mnemo_core::traits::storage::{EdgeStore, EntityStore, RegionStore, UserStore};
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

/// Middleware that injects `CallerContext::admin_bootstrap()` into requests
/// that don't already have a `CallerContext`, simulating auth-disabled mode.
/// Without this the handler fallback `caller_from_extension()` returns
/// `anonymous()` (Read role) and most write/admin endpoints return 403.
/// When `AuthLayer` is present (authed test harness), it sets `CallerContext`
/// before this middleware runs, so the admin fallback is skipped.
async fn inject_admin_caller(
    mut request: Request<Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if request.extensions().get::<CallerContext>().is_none() {
        request
            .extensions_mut()
            .insert(CallerContext::admin_bootstrap());
    }
    next.run(request).await
}

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

/// Build a test app with `require_tls: true` for TLS enforcement tests.
async fn build_test_app_require_tls() -> axum::Router {
    let (_app, mut state, _store) = build_test_harness_with_state_and_prefilter_and_webhooks(
        MetadataPrefilterConfig {
            enabled: false,
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
    state.require_tls = true;
    build_router(state.clone())
        .layer(axum::middleware::from_fn(inject_admin_caller))
        .layer(from_fn_with_state(state, request_context_middleware))
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
        QdrantVectorStore::new(&qdrant_url, &qdrant_prefix, 1536, None)
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
        compression_config: mnemo_retrieval::compression::CompressionConfig::default(),
        compression_stats: Arc::new(mnemo_retrieval::compression::CompressionStats::default()),
        embedding_dimensions: 384,
        hyperbolic_config: mnemo_retrieval::hyperbolic::HyperbolicConfig::default(),
        pipeline_metrics: Arc::new(mnemo_ingest::dag::PipelineMetrics::default()),
        sync_status: Arc::new(tokio::sync::RwLock::new(
            mnemo_core::sync::SyncStatus::disabled(),
        )),
    };

    let app = build_router(state.clone())
        .layer(axum::middleware::from_fn(inject_admin_caller))
        .layer(from_fn_with_state(
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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

    // insufficient evidence should be rejected (only 1 source event)
    let (status, ev1) = json_request(
        &app,
        "POST",
        "/api/v1/agents/promo-agent/experience",
        serde_json::json!({"category": "tone", "signal": "user liked direct tone", "confidence": 0.8}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let single_id = ev1["id"].as_str().unwrap().to_string();

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/agents/promo-agent/promotions",
        serde_json::json!({
            "proposal": "increase directness",
            "candidate_core": {"mission": "new-mission"},
            "reason": "single anecdote",
            "source_event_ids": [single_id]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation_error");

    // non-existent source_event_ids should be rejected even with >= 3 IDs
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/agents/promo-agent/promotions",
        serde_json::json!({
            "proposal": "increase directness",
            "candidate_core": {"mission": "new-mission"},
            "reason": "fabricated evidence",
            "source_event_ids": [uuid::Uuid::now_v7(), uuid::Uuid::now_v7(), uuid::Uuid::now_v7()]
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "fabricated event IDs must be rejected: {body:?}"
    );

    // Create 2 more real experience events so we have 3 total
    let (status, ev2) = json_request(
        &app,
        "POST",
        "/api/v1/agents/promo-agent/experience",
        serde_json::json!({"category": "tone", "signal": "user preferred concise answers", "confidence": 0.9}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, ev3) = json_request(
        &app,
        "POST",
        "/api/v1/agents/promo-agent/experience",
        serde_json::json!({"category": "tone", "signal": "user disliked verbose explanations", "confidence": 0.7}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let source_ids: Vec<String> = vec![
        ev1["id"].as_str().unwrap().to_string(),
        ev2["id"].as_str().unwrap().to_string(),
        ev3["id"].as_str().unwrap().to_string(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
                classification: Default::default(),
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
        QdrantVectorStore::new(&qdrant_url, &qdrant_prefix, 1536, None)
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
        compression_config: mnemo_retrieval::compression::CompressionConfig::default(),
        compression_stats: Arc::new(mnemo_retrieval::compression::CompressionStats::default()),
        embedding_dimensions: 384,
        hyperbolic_config: mnemo_retrieval::hyperbolic::HyperbolicConfig::default(),
        pipeline_metrics: Arc::new(mnemo_ingest::dag::PipelineMetrics::default()),
        sync_status: Arc::new(tokio::sync::RwLock::new(
            mnemo_core::sync::SyncStatus::disabled(),
        )),
    };

    let app1 = build_router(state1.clone())
        .layer(axum::middleware::from_fn(inject_admin_caller))
        .layer(from_fn_with_state(
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
        compression_config: mnemo_retrieval::compression::CompressionConfig::default(),
        compression_stats: Arc::new(mnemo_retrieval::compression::CompressionStats::default()),
        embedding_dimensions: 384,
        hyperbolic_config: mnemo_retrieval::hyperbolic::HyperbolicConfig::default(),
        pipeline_metrics: Arc::new(mnemo_ingest::dag::PipelineMetrics::default()),
        sync_status: Arc::new(tokio::sync::RwLock::new(
            mnemo_core::sync::SyncStatus::disabled(),
        )),
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
        coherence_score: None,
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
            classification: mnemo_core::models::classification::Classification::default(),
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
            classification: mnemo_core::models::classification::Classification::default(),
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
        classification: mnemo_core::models::classification::Classification::default(),
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

// ─── LLM Span Endpoint Tests ──────────────────────────────────────

fn make_api_test_span(
    request_id: Option<&str>,
    user_id: Option<Uuid>,
    operation: &str,
    started_at: chrono::DateTime<chrono::Utc>,
) -> mnemo_core::models::span::LlmSpan {
    mnemo_core::models::span::LlmSpan {
        id: Uuid::now_v7(),
        request_id: request_id.map(String::from),
        user_id,
        provider: "test-provider".into(),
        model: "test-model".into(),
        operation: operation.into(),
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
        latency_ms: 200,
        success: true,
        error: None,
        started_at,
        finished_at: started_at + chrono::Duration::milliseconds(200),
    }
}

#[tokio::test]
async fn test_spans_by_request_from_redis() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let rid = format!("span-req-{}", Uuid::now_v7());
    let user_id = Uuid::now_v7();
    let now = chrono::Utc::now();

    // Inject spans directly into Redis (bypassing in-memory buffer)
    use mnemo_core::traits::storage::SpanStore;
    let span1 = make_api_test_span(Some(&rid), Some(user_id), "extract", now);
    let span2 = make_api_test_span(
        Some(&rid),
        Some(user_id),
        "embed_episode",
        now + chrono::Duration::milliseconds(500),
    );
    state_store.save_span(&span1).await.unwrap();
    state_store.save_span(&span2).await.unwrap();

    // Query via API
    let (status, body) = get_request(&app, &format!("/api/v1/spans/request/{rid}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "spans by request should succeed: {body}"
    );
    assert_eq!(body["request_id"].as_str().unwrap(), rid);
    assert_eq!(body["count"].as_u64().unwrap(), 2);
    assert_eq!(body["total_tokens"].as_u64().unwrap(), 300); // 150 * 2

    let spans = body["spans"].as_array().unwrap();
    assert_eq!(spans.len(), 2);
    // Ascending order
    assert_eq!(spans[0]["operation"].as_str().unwrap(), "extract");
    assert_eq!(spans[1]["operation"].as_str().unwrap(), "embed_episode");
}

#[tokio::test]
async fn test_spans_by_user_from_redis() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let user_id = Uuid::now_v7();
    let now = chrono::Utc::now();

    // Inject spans directly into Redis
    use mnemo_core::traits::storage::SpanStore;
    let span1 = make_api_test_span(None, Some(user_id), "extract", now);
    let span2 = make_api_test_span(
        None,
        Some(user_id),
        "summarize",
        now + chrono::Duration::seconds(1),
    );
    state_store.save_span(&span1).await.unwrap();
    state_store.save_span(&span2).await.unwrap();

    // Query via API
    let (status, body) = get_request(&app, &format!("/api/v1/spans/user/{user_id}?limit=10")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "spans by user should succeed: {body}"
    );
    assert_eq!(body["count"].as_u64().unwrap(), 2);

    let spans = body["spans"].as_array().unwrap();
    assert_eq!(spans.len(), 2);
    // Descending order (newest first)
    assert_eq!(spans[0]["operation"].as_str().unwrap(), "summarize");
    assert_eq!(spans[1]["operation"].as_str().unwrap(), "extract");
}

#[tokio::test]
async fn test_spans_by_request_empty_returns_empty() {
    let app = build_test_app().await;

    let (status, body) = get_request(&app, "/api/v1/spans/request/nonexistent-request-id").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"].as_u64().unwrap(), 0);
    assert_eq!(body["spans"].as_array().unwrap().len(), 0);
}

// ─── PATCH /api/v1/memory/webhooks/:id ────────────────────────────

/// Test: PATCH webhook updates target_url, events, and enabled fields.
#[tokio::test]
async fn test_patch_webhook_updates_fields() {
    let app = build_test_app().await;

    // Create user
    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "patch-webhook-user",
            "external_id": "patch-webhook-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Register a webhook
    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "patch-webhook-user",
            "target_url": "https://original.example/hook",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap();

    // PATCH: update target_url and enabled
    let (status, updated) = json_request(
        &app,
        "PATCH",
        &format!("/api/v1/memory/webhooks/{webhook_id}"),
        serde_json::json!({
            "target_url": "https://updated.example/hook",
            "enabled": false,
            "events": ["fact_added", "fact_superseded"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PATCH should succeed: {updated}");
    assert_eq!(updated["ok"], true);
    assert_eq!(
        updated["webhook"]["target_url"],
        "https://updated.example/hook"
    );
    assert_eq!(updated["webhook"]["enabled"], false);
    let events = updated["webhook"]["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);

    // GET should return updated values
    let (status, got) = get_request(&app, &format!("/api/v1/memory/webhooks/{webhook_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["target_url"], "https://updated.example/hook");
    assert_eq!(got["enabled"], false);
}

/// Test: PATCH webhook with non-existent ID returns 404.
#[tokio::test]
async fn test_patch_webhook_not_found() {
    let app = build_test_app().await;
    let fake_id = Uuid::now_v7();

    let (status, _) = json_request(
        &app,
        "PATCH",
        &format!("/api/v1/memory/webhooks/{fake_id}"),
        serde_json::json!({
            "enabled": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Test: PATCH webhook rejects non-HTTPS target_url when require_tls is enabled.
#[tokio::test]
async fn test_patch_webhook_rejects_http_when_require_tls() {
    let app = build_test_app_require_tls().await;

    // Create user
    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "tls-patch-user",
            "external_id": "tls-patch-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Register webhook with HTTPS (allowed)
    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "tls-patch-user",
            "target_url": "https://secure.example/hook",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap();

    // PATCH: try to downgrade to HTTP — should be rejected
    let (status, body) = json_request(
        &app,
        "PATCH",
        &format!("/api/v1/memory/webhooks/{webhook_id}"),
        serde_json::json!({
            "target_url": "http://insecure.example/hook"
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "PATCH with HTTP URL should fail when require_tls is enabled: {body}"
    );
}

/// Test: PATCH webhook rejects target_url not on domain allowlist.
#[tokio::test]
async fn test_patch_webhook_enforces_domain_allowlist() {
    let app = build_test_app().await;

    // Create user
    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "patch-allowlist-user",
            "external_id": "patch-allowlist-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Set domain allowlist policy
    let (status, _) = json_request(
        &app,
        "PUT",
        "/api/v1/policies/patch-allowlist-user",
        serde_json::json!({
            "webhook_domain_allowlist": ["allowed.example"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Register webhook with allowed domain
    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "patch-allowlist-user",
            "target_url": "https://allowed.example/hook",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap();

    // PATCH: try to change to disallowed domain
    let (status, body) = json_request(
        &app,
        "PATCH",
        &format!("/api/v1/memory/webhooks/{webhook_id}"),
        serde_json::json!({
            "target_url": "https://evil.example/hook"
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "PATCH with disallowed domain should fail: {body}"
    );

    // PATCH: change to allowed subdomain — should succeed
    let (status, updated) = json_request(
        &app,
        "PATCH",
        &format!("/api/v1/memory/webhooks/{webhook_id}"),
        serde_json::json!({
            "target_url": "https://sub.allowed.example/hook"
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "PATCH with allowed subdomain should succeed: {updated}"
    );
    assert_eq!(
        updated["webhook"]["target_url"],
        "https://sub.allowed.example/hook"
    );
}

/// Test: PATCH webhook generates audit trail.
#[tokio::test]
async fn test_patch_webhook_creates_audit_entry() {
    let app = build_test_app().await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "patch-audit-user",
            "external_id": "patch-audit-user",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, registered) = json_request(
        &app,
        "POST",
        "/api/v1/memory/webhooks",
        serde_json::json!({
            "user": "patch-audit-user",
            "target_url": "https://audit.example/hook",
            "events": ["head_advanced"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let webhook_id = registered["webhook"]["id"].as_str().unwrap();

    // PATCH the webhook
    let (status, _) = json_request(
        &app,
        "PATCH",
        &format!("/api/v1/memory/webhooks/{webhook_id}"),
        serde_json::json!({
            "enabled": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Check audit trail
    let (status, audit_body) = get_request(
        &app,
        &format!("/api/v1/memory/webhooks/{webhook_id}/audit?limit=20"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = audit_body["audit"].as_array().unwrap();
    assert!(
        rows.iter().any(|row| row["action"] == "webhook_updated"),
        "Audit trail should contain 'webhook_updated' entry, got: {:?}",
        rows
    );
}

// ── F-10: HMAC signature tamper detection ──────────────────────────

/// Verify that a tampered body produces a different HMAC signature,
/// demonstrating that webhook consumers can detect payload tampering.
#[test]
fn test_hmac_tampered_body_does_not_verify() {
    let secret = "whsec_tamper_test_secret";
    let timestamp = "1700000000";
    let original_body = r#"{"event_type":"head_advanced","user_id":"123"}"#;
    let tampered_body = r#"{"event_type":"head_advanced","user_id":"456"}"#;

    let original_sig = compute_expected_signature(secret, timestamp, original_body);
    let tampered_sig = compute_expected_signature(secret, timestamp, tampered_body);

    // Original signature must verify against original body
    assert!(!original_sig.is_empty(), "Signature should not be empty");
    assert!(
        original_sig.starts_with("t="),
        "Signature must start with t= prefix"
    );
    assert!(
        original_sig.contains(",v1="),
        "Signature must contain ,v1= prefix for the digest"
    );

    // Tampered body must produce a DIFFERENT signature
    assert_ne!(
        original_sig, tampered_sig,
        "Tampered body must produce a different HMAC signature"
    );

    // Wrong secret must also produce a different signature
    let wrong_secret_sig =
        compute_expected_signature("whsec_wrong_secret", timestamp, original_body);
    assert_ne!(
        original_sig, wrong_secret_sig,
        "Wrong signing secret must produce a different HMAC signature"
    );
}

/// Verify that the HMAC signature format follows the expected convention:
/// `t=<unix_timestamp>,v1=<hex_hmac_sha256>`.
#[test]
fn test_hmac_signature_format_correctness() {
    let secret = "whsec_format_test";
    let timestamp = "1700000000";
    let body = r#"{"ok":true}"#;

    let sig = compute_expected_signature(secret, timestamp, body);

    // Parse t= and v1= components
    let parts: Vec<&str> = sig.splitn(2, ",v1=").collect();
    assert_eq!(parts.len(), 2, "Signature must have t= and v1= components");

    let t_part = parts[0];
    assert!(
        t_part.starts_with("t="),
        "First component must be t=<timestamp>"
    );
    let ts = &t_part[2..];
    assert_eq!(ts, timestamp, "Timestamp in signature must match input");

    let hex_digest = parts[1];
    assert_eq!(
        hex_digest.len(),
        64,
        "HMAC-SHA256 hex digest must be 64 characters"
    );
    assert!(
        hex_digest.chars().all(|c| c.is_ascii_hexdigit()),
        "Digest must be valid hex"
    );
}

#[tokio::test]
async fn test_agent_reject_promotion_leaves_identity_core_unchanged() {
    let app = build_test_app().await;

    // Step 1: Auto-create identity at version 1
    let (status, identity_before) = json_request(
        &app,
        "GET",
        "/api/v1/agents/reject-promo-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity_before["version"], 1);
    let core_before = identity_before["core"].clone();

    // Step 2: Create 3 real experience events to back the proposal
    let mut source_ids = Vec::new();
    for signal in &[
        "user preferred terse responses",
        "user asked for more directness",
        "user praised aggressive style",
    ] {
        let (status, ev) = json_request(
            &app,
            "POST",
            "/api/v1/agents/reject-promo-agent/experience",
            serde_json::json!({"category": "style", "signal": signal, "confidence": 0.8}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        source_ids.push(ev["id"].as_str().unwrap().to_string());
    }

    // Step 3: Create a promotion proposal with a different candidate_core
    let (status, proposal) = json_request(
        &app,
        "POST",
        "/api/v1/agents/reject-promo-agent/promotions",
        serde_json::json!({
            "proposal": "change mission to something else",
            "candidate_core": {"mission": "totally-different-mission", "style": "aggressive"},
            "reason": "three events suggest directness works better",
            "source_event_ids": source_ids
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(proposal["status"], "pending");
    let proposal_id = proposal["id"].as_str().unwrap().to_string();

    // Step 4: Reject the proposal
    let (status, rejected) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/reject-promo-agent/promotions/{proposal_id}/reject"),
        serde_json::json!({"reason": "insufficient evidence after human review"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rejected["status"], "rejected");
    assert!(
        rejected["rejected_at"].is_string(),
        "rejected_at must be set"
    );

    // Step 5: Verify identity is completely unchanged
    let (status, identity_after) = json_request(
        &app,
        "GET",
        "/api/v1/agents/reject-promo-agent/identity",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        identity_after["version"], 1,
        "version must stay at 1 after rejection"
    );
    assert_eq!(
        identity_after["core"], core_before,
        "core must be identical after rejection"
    );

    // Step 6: Verify the proposal shows up as rejected in the list
    let (status, proposals) = json_request(
        &app,
        "GET",
        "/api/v1/agents/reject-promo-agent/promotions?limit=10",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let list = proposals.as_array().expect("proposals should be an array");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["status"], "rejected");

    // Step 7: Verify re-rejecting a rejected proposal fails
    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/reject-promo-agent/promotions/{proposal_id}/reject"),
        serde_json::json!({"reason": "double reject attempt"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "re-rejecting must fail: {body:?}"
    );
}

#[tokio::test]
async fn test_gnn_retrieval_feedback_endpoint() {
    let app = build_test_app().await;

    // Feedback with positive IDs should succeed
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/feedback",
        serde_json::json!({
            "positive_entity_ids": [uuid::Uuid::now_v7(), uuid::Uuid::now_v7()],
            "all_entity_ids": [
                uuid::Uuid::now_v7(),
                uuid::Uuid::now_v7(),
                uuid::Uuid::now_v7(),
                uuid::Uuid::now_v7(),
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "feedback should succeed: {body:?}");
    assert_eq!(body["accepted"], true);
    assert_eq!(body["positive_count"], 2);

    // Feedback with empty positive_entity_ids should fail validation
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/memory/feedback",
        serde_json::json!({
            "positive_entity_ids": [],
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "empty positives must fail: {body:?}"
    );
}

// ─── EWC++ Experience Weight Consolidation ─────────────────────────

#[tokio::test]
async fn test_ewc_experience_fisher_importance_computed_on_create() {
    let app = build_test_app().await;

    // First event in a new category should get fisher_importance = 1.0
    let (status, first) = json_request(
        &app,
        "POST",
        "/api/v1/agents/ewc-test-agent/experience",
        serde_json::json!({
            "category": "domain_skill",
            "signal": "user praised billing knowledge",
            "confidence": 0.9,
            "weight": 0.8
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let fisher_first = first["fisher_importance"].as_f64().unwrap();
    assert!(
        (fisher_first - 1.0).abs() < 0.01,
        "first event in category should have fisher ~1.0, got {}",
        fisher_first
    );

    // Second event in same category should have lower fisher importance
    let (status, second) = json_request(
        &app,
        "POST",
        "/api/v1/agents/ewc-test-agent/experience",
        serde_json::json!({
            "category": "domain_skill",
            "signal": "user asked follow-up billing question",
            "confidence": 0.85,
            "weight": 0.7
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let fisher_second = second["fisher_importance"].as_f64().unwrap();
    assert!(
        fisher_second < fisher_first,
        "second event fisher ({}) should be < first ({})",
        fisher_second,
        fisher_first
    );
    assert!(
        fisher_second > 0.0,
        "second event fisher should still be positive: {}",
        fisher_second
    );
}

#[tokio::test]
async fn test_ewc_experience_importance_endpoint() {
    let app = build_test_app().await;

    // Create events in two categories
    for (cat, sig) in &[
        ("tone", "user prefers formal"),
        ("tone", "user dislikes slang"),
        ("domain", "user is a billing expert"),
    ] {
        let (status, _) = json_request(
            &app,
            "POST",
            "/api/v1/agents/ewc-importance-agent/experience",
            serde_json::json!({
                "category": cat,
                "signal": sig,
                "confidence": 0.85,
                "weight": 0.7
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // Fetch importance ranking
    let (status, ranked) = json_request(
        &app,
        "GET",
        "/api/v1/agents/ewc-importance-agent/experience/importance",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let list = ranked.as_array().expect("should be array");
    assert_eq!(list.len(), 3);

    // Should be sorted by fisher_importance descending
    let fishers: Vec<f64> = list
        .iter()
        .map(|e| e["fisher_importance"].as_f64().unwrap())
        .collect();
    for i in 1..fishers.len() {
        assert!(
            fishers[i - 1] >= fishers[i],
            "importance should be descending: {:?}",
            fishers
        );
    }

    // First event (domain, sole in category) should have highest fisher
    assert_eq!(list[0]["category"], "domain");
    assert!(
        (fishers[0] - 1.0).abs() < 0.01,
        "sole-category event should have fisher ~1.0"
    );

    // Each entry should have the expected fields
    for entry in list {
        assert!(entry["id"].is_string());
        assert!(entry["fisher_importance"].is_number());
        assert!(entry["effective_weight"].is_number());
        assert!(entry["raw_weight"].is_number());
        assert!(entry["confidence"].is_number());
    }
}

#[tokio::test]
async fn test_ewc_high_fisher_events_resist_decay_in_context() {
    let app = build_test_app().await;

    // Setup: update identity with a mission
    let (status, _) = json_request(
        &app,
        "PUT",
        "/api/v1/agents/ewc-decay-agent/identity",
        serde_json::json!({"core": {"mission": "billing assistant"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Create an event — it gets fisher=1.0 (first in category)
    let (status, event) = json_request(
        &app,
        "POST",
        "/api/v1/agents/ewc-decay-agent/experience",
        serde_json::json!({
            "category": "core_skill",
            "signal": "master of refund processing",
            "confidence": 0.95,
            "weight": 1.0
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let fisher = event["fisher_importance"].as_f64().unwrap();
    assert!(
        (fisher - 1.0).abs() < 0.01,
        "first event should have fisher ~1.0"
    );

    // The effective_weight with fisher=1.0 should be > raw weight*confidence
    // because protection = 1 + 1.0 * 2.0 = 3.0 (for fresh event, decay=1.0)
    // effective = 1.0 * 0.95 * 1.0 * 3.0 = 2.85
    let importance_resp = json_request(
        &app,
        "GET",
        "/api/v1/agents/ewc-decay-agent/experience/importance",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(importance_resp.0, StatusCode::OK);
    let list = importance_resp.1.as_array().unwrap();
    assert_eq!(list.len(), 1);
    let ew = list[0]["effective_weight"].as_f64().unwrap();
    let raw = list[0]["raw_weight"].as_f64().unwrap();
    let conf = list[0]["confidence"].as_f64().unwrap();
    // With fisher=1.0 and EWC_LAMBDA=2.0, effective should be ~3x the base
    assert!(
        ew > raw * conf * 2.5,
        "effective_weight ({}) should be ~3x raw*conf ({})",
        ew,
        raw * conf
    );
}

// ─── Temporal Tensor Compression ───────────────────────────────────

#[tokio::test]
async fn test_ops_compression_endpoint_returns_stats() {
    let app = build_test_app().await;

    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/ops/compression",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Check structure
    assert_eq!(body["enabled"], false); // disabled by default in test config
    assert_eq!(body["dimensions"], 384);
    assert!(body["total_points"].is_number());
    assert!(body["tiers"]["full"]["count"].is_number());
    assert!(body["tiers"]["half"]["count"].is_number());
    assert!(body["tiers"]["int8"]["count"].is_number());
    assert!(body["tiers"]["binary"]["count"].is_number());
    assert!(body["storage"]["estimated_bytes"].is_number());
    assert!(body["storage"]["uncompressed_bytes"].is_number());
    assert!(body["storage"]["savings_percent"].is_number());
    assert!(body["sweep"]["interval_secs"].is_number());
    assert!(body["sweep"]["total_sweeps"].is_number());
    assert!(body["sweep"]["last_sweep_at"].is_string());
}

#[tokio::test]
async fn test_ops_compression_stats_reflect_tier_counts() {
    let app = build_test_app().await;

    // Initially all zeros
    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/ops/compression",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_points"], 0);
    assert_eq!(body["tiers"]["full"]["count"], 0);
    assert_eq!(body["sweep"]["total_sweeps"], 0);
    assert_eq!(body["sweep"]["last_sweep_at"], "never");
}

// ═══════════════════════════════════════════════════════════════════════
// Coherence scoring endpoint
// ═══════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_coherence_empty_user_returns_high_score() {
    let app = build_test_app().await;

    // Create user with no entities/edges
    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "coherence-empty-user",
            "external_id": "coherence-empty",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = user["id"].as_str().unwrap();

    let (status, body) = get_request(&app, &format!("/api/v1/users/{}/coherence", user_id)).await;
    assert_eq!(status, StatusCode::OK);

    // Empty graph should be vacuously coherent (>=0.9)
    let score = body["score"].as_f64().unwrap();
    assert!(
        score >= 0.9,
        "empty graph should be vacuously coherent: {}",
        score
    );
    assert_eq!(body["diagnostics"]["total_entities"], 0);
    assert_eq!(body["diagnostics"]["total_edges"], 0);
    assert!(body["recommendations"].is_array());
}

#[tokio::test]
async fn test_coherence_healthy_graph_scores_well() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    // Create user
    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "coherence-healthy-user",
            "external_id": "coherence-healthy",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = Uuid::parse_str(user["id"].as_str().unwrap()).unwrap();

    // Create entities: Person + Organization + Location (diverse types)
    let episode_id = Uuid::now_v7();
    let alice = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Alice".to_string(),
                entity_type: EntityType::Person,
                summary: Some("Software engineer".to_string()),
                classification: Default::default(),
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let acme = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Acme Corp".to_string(),
                entity_type: EntityType::Organization,
                summary: Some("Tech company".to_string()),
                classification: Default::default(),
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let nyc = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "New York".to_string(),
                entity_type: EntityType::Location,
                summary: Some("City".to_string()),
                classification: Default::default(),
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();

    // Create edges with high confidence (no conflicts)
    let now = Utc::now();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Alice".to_string(),
                target_name: "Acme Corp".to_string(),
                label: "works_at".to_string(),
                fact: "Alice works at Acme Corp".to_string(),
                confidence: 0.9,
                valid_at: Some(now - chrono::Duration::days(10)),
                classification: Default::default(),
            },
            user_id,
            alice.id,
            acme.id,
            episode_id,
            now,
        ))
        .await
        .unwrap();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Acme Corp".to_string(),
                target_name: "New York".to_string(),
                label: "located_in".to_string(),
                fact: "Acme Corp is located in New York".to_string(),
                confidence: 0.95,
                valid_at: Some(now - chrono::Duration::days(30)),
                classification: Default::default(),
            },
            user_id,
            acme.id,
            nyc.id,
            episode_id,
            now,
        ))
        .await
        .unwrap();

    let (status, body) = get_request(&app, &format!("/api/v1/users/{}/coherence", user_id)).await;
    assert_eq!(status, StatusCode::OK);

    let score = body["score"].as_f64().unwrap();
    assert!(
        score > 0.5,
        "healthy graph should score reasonably: {}",
        score
    );
    assert_eq!(body["diagnostics"]["total_entities"], 3);
    assert_eq!(body["diagnostics"]["active_edges"], 2);
    assert_eq!(body["diagnostics"]["conflicting_groups"], 0);
    assert!(body["entity_coherence"].as_f64().unwrap() > 0.0);
    assert!(body["fact_coherence"].as_f64().unwrap() > 0.0);
    assert!(body["temporal_coherence"].as_f64().unwrap() > 0.0);
    assert!(body["structural_coherence"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn test_coherence_detects_fact_conflicts() {
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
            "name": "coherence-conflict-user",
            "external_id": "coherence-conflict",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = Uuid::parse_str(user["id"].as_str().unwrap()).unwrap();

    let episode_id = Uuid::now_v7();
    let alice = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Alice".to_string(),
                entity_type: EntityType::Person,
                summary: None,
                classification: Default::default(),
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let acme = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Acme Corp".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
                classification: Default::default(),
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();
    let globex = state_store
        .create_entity(Entity::from_extraction(
            &ExtractedEntity {
                name: "Globex Corp".to_string(),
                entity_type: EntityType::Organization,
                summary: None,
                classification: Default::default(),
            },
            user_id,
            episode_id,
        ))
        .await
        .unwrap();

    // Create conflicting edges: Alice works_at BOTH Acme AND Globex simultaneously
    let now = Utc::now();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Alice".to_string(),
                target_name: "Acme Corp".to_string(),
                label: "works_at".to_string(),
                fact: "Alice works at Acme Corp".to_string(),
                confidence: 0.9,
                valid_at: Some(now - chrono::Duration::days(5)),
                classification: Default::default(),
            },
            user_id,
            alice.id,
            acme.id,
            episode_id,
            now,
        ))
        .await
        .unwrap();
    state_store
        .create_edge(Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "Alice".to_string(),
                target_name: "Globex Corp".to_string(),
                label: "works_at".to_string(),
                fact: "Alice works at Globex Corp".to_string(),
                confidence: 0.7,
                valid_at: Some(now - chrono::Duration::days(2)),
                classification: Default::default(),
            },
            user_id,
            alice.id,
            globex.id,
            episode_id,
            now,
        ))
        .await
        .unwrap();

    let (status, body) = get_request(&app, &format!("/api/v1/users/{}/coherence", user_id)).await;
    assert_eq!(status, StatusCode::OK);

    // Fact coherence should be reduced due to conflict
    let fact_coherence = body["fact_coherence"].as_f64().unwrap();
    assert!(
        fact_coherence < 1.0,
        "conflicting facts should reduce fact coherence: {}",
        fact_coherence
    );
    assert_eq!(body["diagnostics"]["conflicting_groups"], 1);
    // Should have a recommendation about conflicts
    let recs = body["recommendations"].as_array().unwrap();
    assert!(
        recs.iter()
            .any(|r| r.as_str().unwrap().contains("conflicting")),
        "should recommend resolving conflicts: {:?}",
        recs
    );
}

#[tokio::test]
async fn test_coherence_user_by_external_id() {
    let app = build_test_app().await;

    let (status, _user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "coherence-extid-user",
            "external_id": "coherence-ext-lookup",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Access coherence by external_id
    let (status, body) = get_request(&app, "/api/v1/users/coherence-ext-lookup/coherence").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["score"].as_f64().is_some());
}

#[tokio::test]
async fn test_coherence_nonexistent_user_404() {
    let app = build_test_app().await;

    let (status, _body) =
        get_request(&app, &format!("/api/v1/users/{}/coherence", Uuid::now_v7())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_coherence_response_all_fields_present() {
    let app = build_test_app().await;

    let (status, user) = json_request(
        &app,
        "POST",
        "/api/v1/users",
        serde_json::json!({
            "name": "coherence-fields-user",
            "external_id": "coherence-fields",
            "metadata": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = get_request(
        &app,
        &format!("/api/v1/users/{}/coherence", user["id"].as_str().unwrap()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Verify all top-level fields are present
    assert!(body["user_id"].is_string(), "user_id should be present");
    assert!(body["score"].is_number(), "score should be present");
    assert!(
        body["entity_coherence"].is_number(),
        "entity_coherence should be present"
    );
    assert!(
        body["fact_coherence"].is_number(),
        "fact_coherence should be present"
    );
    assert!(
        body["temporal_coherence"].is_number(),
        "temporal_coherence should be present"
    );
    assert!(
        body["structural_coherence"].is_number(),
        "structural_coherence should be present"
    );
    assert!(
        body["recommendations"].is_array(),
        "recommendations should be present"
    );
    assert!(
        body["diagnostics"].is_object(),
        "diagnostics should be present"
    );

    // Verify diagnostics sub-fields
    let diag = &body["diagnostics"];
    assert!(diag["total_entities"].is_number());
    assert!(diag["total_edges"].is_number());
    assert!(diag["active_edges"].is_number());
    assert!(diag["invalidated_edges"].is_number());
    assert!(diag["conflicting_groups"].is_number());
    assert!(diag["communities_detected"].is_number());
    assert!(diag["isolated_entities"].is_number());
    assert!(diag["recent_supersessions"].is_number());
    assert!(diag["recent_corroborations"].is_number());

    // Score should be in [0, 1]
    let score = body["score"].as_f64().unwrap();
    assert!(
        (0.0..=1.0).contains(&score),
        "score must be in [0,1]: {}",
        score
    );
}

// =============================================================================
// RBAC-01: Scoped API Key CRUD integration tests
// =============================================================================

/// Builds a test app with auth enabled, bootstrap admin keys, and Redis-backed
/// scoped key support. Returns (app, state_store, admin_key) so tests can
/// authenticate as admin and also exercise scoped key lookups.
async fn build_authed_test_app_with_store(
    admin_keys: Vec<String>,
) -> (axum::Router, Arc<RedisStateStore>, String) {
    let (base_app, _state, state_store) = build_test_harness_with_state_and_prefilter_and_webhooks(
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

    let admin_key = admin_keys[0].clone();

    // Wrap with auth layer using with_keys_and_store for scoped key support
    let app = base_app.layer(AuthLayer::new(AuthConfig::with_keys_and_store(
        admin_keys,
        state_store.clone(),
    )));

    (app, state_store, admin_key)
}

// ---- RBAC-01a: Create API key ----

#[tokio::test]
async fn rbac01a_create_api_key_returns_raw_key_and_metadata() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({
            "name": "test-read-key",
            "role": "read",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "create key failed: {:?}", body);

    // raw_key must start with "mnk_" and be shown exactly once
    let raw_key = body["raw_key"].as_str().unwrap();
    assert!(
        raw_key.starts_with("mnk_"),
        "raw_key should start with mnk_"
    );
    assert_eq!(raw_key.len(), 68, "mnk_ + 64 hex chars = 68");

    // Metadata fields
    assert!(body["id"].is_string());
    assert_eq!(body["name"].as_str().unwrap(), "test-read-key");
    assert_eq!(body["role"].as_str().unwrap(), "read");
    assert!(!body["revoked"].as_bool().unwrap());
    assert!(body["key_prefix"].is_string());
    assert!(body["created_at"].is_string());
}

#[tokio::test]
async fn rbac01a_create_api_key_with_scope() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    let user_id = Uuid::from_u128(42);
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({
            "name": "scoped-write-key",
            "role": "write",
            "scope": {
                "allowed_user_ids": [user_id],
                "max_classification": "confidential",
            },
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CREATED,
        "create scoped key failed: {:?}",
        body
    );
    assert_eq!(body["role"].as_str().unwrap(), "write");
    assert_eq!(
        body["scope"]["allowed_user_ids"][0].as_str().unwrap(),
        user_id.to_string()
    );
    assert_eq!(
        body["scope"]["max_classification"].as_str().unwrap(),
        "confidential"
    );
}

#[tokio::test]
async fn rbac01a_create_api_key_rejects_empty_name() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({
            "name": "   ",
            "role": "read",
        }),
    )
    .await;

    // Empty/whitespace-only name should be rejected
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "blank name should fail: {:?}",
        body
    );
}

#[tokio::test]
async fn rbac01a_create_api_key_without_auth_returns_401() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // No auth header at all
    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/keys",
        serde_json::json!({
            "name": "should-fail",
            "role": "read",
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "no-auth should 401: {:?}",
        body
    );
}

// ---- RBAC-01b: List API keys ----

#[tokio::test]
async fn rbac01b_list_api_keys_returns_created_keys() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create two keys
    for name in ["list-key-1", "list-key-2"] {
        let (status, _) = json_request_with_header(
            &app,
            "POST",
            "/api/v1/keys",
            "authorization",
            "Bearer rbac-admin-key",
            serde_json::json!({ "name": name, "role": "read" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // List keys
    let (status, body) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "list keys failed: {:?}", body);
    let keys = body["keys"].as_array().unwrap();
    assert!(
        keys.len() >= 2,
        "should have at least 2 keys, got {}",
        keys.len()
    );

    // Verify no key_hash is exposed
    for key in keys {
        assert!(
            key.get("key_hash").is_none(),
            "key_hash must not be exposed in list"
        );
    }
}

#[tokio::test]
async fn rbac01b_list_api_keys_respects_limit() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create 3 keys
    for i in 0..3 {
        let (status, _) = json_request_with_header(
            &app,
            "POST",
            "/api/v1/keys",
            "authorization",
            "Bearer rbac-admin-key",
            serde_json::json!({ "name": format!("limit-key-{}", i), "role": "read" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // List with limit=2
    let (status, body) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/keys?limit=2",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "list with limit failed: {:?}", body);
    let keys = body["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 2, "limit=2 should return exactly 2 keys");
}

// ---- RBAC-01c: Revoke API key ----

#[tokio::test]
async fn rbac01c_revoke_api_key_sets_revoked_flag() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create a key
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({ "name": "to-revoke", "role": "write" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let key_id = create_body["id"].as_str().unwrap();

    // Revoke it
    let (status, _body) = json_request_with_header(
        &app,
        "DELETE",
        &format!("/api/v1/keys/{}", key_id),
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "revoke should return 204");

    // List and verify the key is revoked
    let (status, list_body) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let keys = list_body["keys"].as_array().unwrap();
    let revoked_key = keys
        .iter()
        .find(|k| k["id"].as_str().unwrap() == key_id)
        .unwrap();
    assert!(
        revoked_key["revoked"].as_bool().unwrap(),
        "key should be revoked"
    );
}

#[tokio::test]
async fn rbac01c_revoke_nonexistent_key_returns_404() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    let fake_id = Uuid::from_u128(999);
    let (status, _body) = json_request_with_header(
        &app,
        "DELETE",
        &format!("/api/v1/keys/{}", fake_id),
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "nonexistent key should 404");
}

// ---- RBAC-01d: Rotate API key ----

#[tokio::test]
async fn rbac01d_rotate_api_key_revokes_old_creates_new() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create a key
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({ "name": "to-rotate", "role": "write" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let old_key_id = create_body["id"].as_str().unwrap().to_string();
    let old_raw_key = create_body["raw_key"].as_str().unwrap().to_string();

    // Rotate
    let (status, rotate_body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/keys/{}/rotate", old_key_id),
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "rotate should return 201: {:?}",
        rotate_body
    );

    let new_key_id = rotate_body["id"].as_str().unwrap();
    let new_raw_key = rotate_body["raw_key"].as_str().unwrap();

    // New key should differ from old
    assert_ne!(old_key_id, new_key_id, "new key should have different id");
    assert_ne!(old_raw_key, new_raw_key, "new raw key should differ");

    // New key should have same name and role
    assert_eq!(rotate_body["name"].as_str().unwrap(), "to-rotate");
    assert_eq!(rotate_body["role"].as_str().unwrap(), "write");
    assert!(
        !rotate_body["revoked"].as_bool().unwrap(),
        "new key should not be revoked"
    );

    // Old key should now be revoked (verify via list)
    let (status, list_body) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let keys = list_body["keys"].as_array().unwrap();
    let old = keys
        .iter()
        .find(|k| k["id"].as_str().unwrap() == old_key_id)
        .unwrap();
    assert!(
        old["revoked"].as_bool().unwrap(),
        "old key must be revoked after rotation"
    );
}

#[tokio::test]
async fn rbac01d_rotate_nonexistent_key_returns_404() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    let fake_id = Uuid::from_u128(888);
    let (status, _body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/keys/{}/rotate", fake_id),
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "rotate nonexistent should 404"
    );
}

// =============================================================================
// RBAC-02: Scoped API Key falsification / security tests
// =============================================================================

// ---- RBAC-02a: Scoped key authentication via middleware ----

#[tokio::test]
async fn rbac02a_scoped_key_authenticates_via_middleware() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create a write-scoped key
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({ "name": "write-key", "role": "write" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let raw_key = create_body["raw_key"].as_str().unwrap().to_string();

    // Use the scoped key to write memory (should succeed — Write >= Write)
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        "authorization",
        &format!("Bearer {}", raw_key),
        serde_json::json!({
            "user": format!("rbac02a_user_{}", Uuid::now_v7()),
            "session": "s1",
            "text": "scoped key write test",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "scoped write key should be able to ingest: {:?}",
        body
    );
}

// ---- RBAC-02b: Role escalation prevention ----

#[tokio::test]
async fn rbac02b_read_key_cannot_access_admin_endpoints() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create a read-only key
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({ "name": "reader", "role": "read" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let read_key = create_body["raw_key"].as_str().unwrap().to_string();

    // Try to list keys (Admin-only) with the read key — should fail
    let (status, body) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/keys",
        "authorization",
        &format!("Bearer {}", read_key),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "read key should not list keys: {:?}",
        body
    );

    // Try to create a key (Admin-only) with the read key — should fail
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        &format!("Bearer {}", read_key),
        serde_json::json!({ "name": "escalated", "role": "admin" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "read key should not create keys: {:?}",
        body
    );
}

#[tokio::test]
async fn rbac02b_write_key_cannot_access_admin_endpoints() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create a write key
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({ "name": "writer", "role": "write" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let write_key = create_body["raw_key"].as_str().unwrap().to_string();

    // Try to list keys with write key — should fail (Write < Admin)
    let (status, body) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/keys",
        "authorization",
        &format!("Bearer {}", write_key),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "write key should not list keys: {:?}",
        body
    );
}

// ---- RBAC-02c: Revoked key rejection ----

#[tokio::test]
async fn rbac02c_revoked_key_cannot_authenticate() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create and then revoke a key
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({ "name": "to-revoke-auth", "role": "write" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let raw_key = create_body["raw_key"].as_str().unwrap().to_string();
    let key_id = create_body["id"].as_str().unwrap().to_string();

    // Revoke
    let (status, _) = json_request_with_header(
        &app,
        "DELETE",
        &format!("/api/v1/keys/{}", key_id),
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Wait briefly to let cache expire (cache TTL is 30s, but for a freshly revoked key
    // the cache entry was set before revocation, so we need the cache to be refreshed).
    // The middleware checks the store on cache miss. Since we just created and revoked,
    // the cache may still have the old active entry. We need to wait for cache expiry
    // or the key was never cached (first auth attempt after revocation = cache miss).
    // Actually: the key was created but never used to authenticate, so it's not cached.
    // First use after revocation = fresh lookup = will see revoked = reject.

    // Try to use the revoked key
    let (status, _body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        "authorization",
        &format!("Bearer {}", raw_key),
        serde_json::json!({
            "user": "revoked-test",
            "session": "s1",
            "text": "should fail",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "revoked key must be rejected"
    );
}

// ---- RBAC-02d: Expired key rejection ----

#[tokio::test]
async fn rbac02d_expired_key_cannot_authenticate() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create a key that is already expired (expires_at in the past)
    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({
            "name": "expired-key",
            "role": "write",
            "expires_at": past.to_rfc3339(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let raw_key = create_body["raw_key"].as_str().unwrap().to_string();

    // Try to use the expired key
    let (status, _body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        "authorization",
        &format!("Bearer {}", raw_key),
        serde_json::json!({
            "user": "expired-test",
            "session": "s1",
            "text": "should fail",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "expired key must be rejected"
    );
}

// ---- RBAC-02e: Rotated key — old key stops working, new key works ----

#[tokio::test]
async fn rbac02e_rotated_old_key_rejected_new_key_works() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create a key
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({ "name": "rotate-auth-test", "role": "write" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let old_raw = create_body["raw_key"].as_str().unwrap().to_string();
    let key_id = create_body["id"].as_str().unwrap().to_string();

    // Rotate
    let (status, rotate_body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/keys/{}/rotate", key_id),
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let new_raw = rotate_body["raw_key"].as_str().unwrap().to_string();

    // Old key should fail (revoked by rotation)
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        "authorization",
        &format!("Bearer {}", old_raw),
        serde_json::json!({
            "user": "rotate-test-old",
            "session": "s1",
            "text": "should fail",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "old key after rotation must be rejected"
    );

    // New key should work
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        "authorization",
        &format!("Bearer {}", new_raw),
        serde_json::json!({
            "user": format!("rbac02e_user_{}", Uuid::now_v7()),
            "session": "s1",
            "text": "new key write",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "new rotated key should work: {:?}",
        body
    );
}

// ---- RBAC-02f: Fabricated / invalid key rejected ----

#[tokio::test]
async fn rbac02f_fabricated_key_rejected() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // A key that looks valid (mnk_ prefix) but was never created
    let (status, _body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/memory",
        "authorization",
        "Bearer mnk_0000000000000000000000000000000000000000000000000000000000000000",
        serde_json::json!({
            "user": "fabricated-test",
            "session": "s1",
            "text": "should fail",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "fabricated key must be rejected"
    );
}

// ---- RBAC-02g: Key name length validation ----

#[tokio::test]
async fn rbac02g_key_name_too_long_rejected() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    let long_name = "x".repeat(200);
    let (status, _body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({
            "name": long_name,
            "role": "read",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "name >128 chars should fail"
    );
}

// =============================================================================
// CLASSIFY-01: Data Classification Labels integration tests
// =============================================================================

use mnemo_core::models::classification::Classification;

// ---- CLASSIFY-01a: Edge classification defaults to Internal ----

#[tokio::test]
async fn classify01a_edge_defaults_to_internal() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let user_id = Uuid::now_v7();
    let src_entity = Uuid::now_v7();
    let tgt_entity = Uuid::now_v7();
    let now = chrono::Utc::now();

    let edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "Alice".into(),
            target_name: "Bob".into(),
            label: "knows".into(),
            fact: "Alice knows Bob".into(),
            confidence: 0.9,
            valid_at: None,
            classification: Default::default(),
        },
        user_id,
        src_entity,
        tgt_entity,
        Uuid::now_v7(),
        now,
    );
    let created = state_store.create_edge(edge).await.unwrap();

    let (status, body) = get_request(&app, &format!("/api/v1/edges/{}", created.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["classification"].as_str().unwrap(), "internal");
}

// ---- CLASSIFY-01b: Entity classification defaults to Internal ----

#[tokio::test]
async fn classify01b_entity_defaults_to_internal() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "TestCo".into(),
            entity_type: EntityType::Organization,
            summary: None,
            classification: Default::default(),
        },
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let created = state_store.create_entity(entity).await.unwrap();

    let (status, body) = get_request(&app, &format!("/api/v1/entities/{}", created.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["classification"].as_str().unwrap(), "internal");
}

// ---- CLASSIFY-01c: PATCH entity classification ----

#[tokio::test]
async fn classify01c_patch_entity_classification() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "PatchTarget".into(),
            entity_type: EntityType::Concept,
            summary: None,
            classification: Default::default(),
        },
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let created = state_store.create_entity(entity).await.unwrap();
    assert_eq!(created.classification, Classification::Internal);

    // PATCH to Confidential
    let (status, body) = json_request(
        &app,
        "PATCH",
        &format!("/api/v1/entities/{}/classification", created.id),
        serde_json::json!({ "classification": "confidential" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "patch entity classification failed: {:?}",
        body
    );
    assert_eq!(body["classification"].as_str().unwrap(), "confidential");

    // Verify via GET
    let (status, body) = get_request(&app, &format!("/api/v1/entities/{}", created.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["classification"].as_str().unwrap(), "confidential");
}

// ---- CLASSIFY-01d: PATCH edge classification ----

#[tokio::test]
async fn classify01d_patch_edge_classification() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let now = chrono::Utc::now();
    let edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "X".into(),
            target_name: "Y".into(),
            label: "related".into(),
            fact: "X relates to Y".into(),
            confidence: 0.8,
            valid_at: None,
            classification: Default::default(),
        },
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        now,
    );
    let created = state_store.create_edge(edge).await.unwrap();

    // PATCH to Restricted
    let (status, body) = json_request(
        &app,
        "PATCH",
        &format!("/api/v1/edges/{}/classification", created.id),
        serde_json::json!({ "classification": "restricted" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "patch edge classification failed: {:?}",
        body
    );
    assert_eq!(body["classification"].as_str().unwrap(), "restricted");
}

// ---- CLASSIFY-01e: EdgeFilter max_classification enforcement ----

#[tokio::test]
async fn classify01e_edge_filter_max_classification() {
    let (_app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let user_id = Uuid::now_v7();
    let src = Uuid::now_v7();
    let tgt = Uuid::now_v7();
    let now = chrono::Utc::now();

    // Create edges at different classification levels
    for (label, class) in [
        ("public_fact", Classification::Public),
        ("internal_fact", Classification::Internal),
        ("confidential_fact", Classification::Confidential),
        ("restricted_fact", Classification::Restricted),
    ] {
        let mut edge = Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "A".into(),
                target_name: "B".into(),
                label: label.into(),
                fact: format!("{} fact", label),
                confidence: 0.9,
                valid_at: None,
                classification: class,
            },
            user_id,
            src,
            tgt,
            Uuid::now_v7(),
            now,
        );
        edge.classification = class;
        state_store.create_edge(edge).await.unwrap();
    }

    // Filter with max_classification = Internal — should get Public + Internal only
    let filter = mnemo_core::models::edge::EdgeFilter {
        max_classification: Some(Classification::Internal),
        ..Default::default()
    };
    let results = state_store.query_edges(user_id, filter).await.unwrap();
    assert_eq!(
        results.len(),
        2,
        "Internal filter should return 2 edges, got {}",
        results.len()
    );
    for edge in &results {
        assert!(
            edge.classification <= Classification::Internal,
            "edge {} has classification {:?}, expected <= Internal",
            edge.label,
            edge.classification
        );
    }

    // Filter with max_classification = Public — should get only Public
    let filter = mnemo_core::models::edge::EdgeFilter {
        max_classification: Some(Classification::Public),
        ..Default::default()
    };
    let results = state_store.query_edges(user_id, filter).await.unwrap();
    assert_eq!(results.len(), 1, "Public filter should return 1 edge");
    assert_eq!(results[0].classification, Classification::Public);

    // Filter with max_classification = Restricted — should get all 4
    let filter = mnemo_core::models::edge::EdgeFilter {
        max_classification: Some(Classification::Restricted),
        ..Default::default()
    };
    let results = state_store.query_edges(user_id, filter).await.unwrap();
    assert_eq!(
        results.len(),
        4,
        "Restricted filter should return all 4 edges"
    );

    // No classification filter — should also get all 4
    let filter = mnemo_core::models::edge::EdgeFilter::default();
    let results = state_store.query_edges(user_id, filter).await.unwrap();
    assert_eq!(results.len(), 4, "No filter should return all 4 edges");
}

// ---- CLASSIFY-01f: Entity list max_classification query param ----

#[tokio::test]
async fn classify01f_entity_list_max_classification() {
    let (app, state_store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: true,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let user_id = Uuid::now_v7();

    // Create entities at different classification levels
    for (name, class) in [
        ("PublicEntity", Classification::Public),
        ("InternalEntity", Classification::Internal),
        ("ConfidentialEntity", Classification::Confidential),
    ] {
        let mut entity = Entity::from_extraction(
            &ExtractedEntity {
                name: name.into(),
                entity_type: EntityType::Concept,
                summary: None,
                classification: class,
            },
            user_id,
            Uuid::now_v7(),
        );
        entity.classification = class;
        state_store.create_entity(entity).await.unwrap();
    }

    // List with max_classification=internal — should get 2 (Public + Internal)
    let (status, body) = get_request(
        &app,
        &format!(
            "/api/v1/users/{}/entities?max_classification=internal",
            user_id
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "list entities with classification filter: {:?}",
        body
    );
    let entities = body["data"].as_array().unwrap();
    assert_eq!(
        entities.len(),
        2,
        "Internal filter should return 2 entities, got {}",
        entities.len()
    );

    // List with max_classification=public — should get 1
    let (status, body) = get_request(
        &app,
        &format!(
            "/api/v1/users/{}/entities?max_classification=public",
            user_id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entities = body["data"].as_array().unwrap();
    assert_eq!(entities.len(), 1, "Public filter should return 1 entity");
}

// ---- CLASSIFY-01g: Backward compatibility — missing field defaults to Internal ----

#[tokio::test]
async fn classify01g_backward_compat_missing_classification_defaults_internal() {
    // Simulate deserializing an edge from Redis that was stored before v0.6.0
    // (i.e., no "classification" field in the JSON)
    let json = serde_json::json!({
        "id": "00000000-0000-0000-0000-000000000001",
        "user_id": "00000000-0000-0000-0000-000000000002",
        "source_entity_id": "00000000-0000-0000-0000-000000000003",
        "target_entity_id": "00000000-0000-0000-0000-000000000004",
        "label": "likes",
        "fact": "old fact",
        "valid_at": "2024-01-01T00:00:00Z",
        "ingested_at": "2024-01-01T00:00:00Z",
        "source_episode_id": "00000000-0000-0000-0000-000000000005",
        "confidence": 0.9,
        "corroboration_count": 1,
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z"
    });
    let edge: Edge = serde_json::from_value(json).unwrap();
    assert_eq!(
        edge.classification,
        Classification::Internal,
        "missing classification should default to Internal"
    );
}

// ---- CLASSIFY-01h: Classification from extraction flows through ----

#[tokio::test]
async fn classify01h_extraction_classification_flows_through() {
    let rel = ExtractedRelationship {
        source_name: "John".into(),
        target_name: "Acme Bank".into(),
        label: "has_account".into(),
        fact: "John has account at Acme Bank".into(),
        confidence: 0.95,
        valid_at: None,
        classification: Classification::Restricted,
    };
    let edge = Edge::from_extraction(
        &rel,
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        Uuid::from_u128(3),
        Uuid::from_u128(4),
        chrono::Utc::now(),
    );
    assert_eq!(
        edge.classification,
        Classification::Restricted,
        "LLM-suggested classification should flow through"
    );
}

// ---- CLASSIFY-01i: Classification::from_str_flexible ----

#[tokio::test]
async fn classify01i_from_str_flexible_parsing() {
    assert_eq!(
        Classification::from_str_flexible("public"),
        Classification::Public
    );
    assert_eq!(
        Classification::from_str_flexible("PUBLIC"),
        Classification::Public
    );
    assert_eq!(
        Classification::from_str_flexible("  confidential  "),
        Classification::Confidential
    );
    assert_eq!(
        Classification::from_str_flexible("restricted"),
        Classification::Restricted
    );
    assert_eq!(
        Classification::from_str_flexible("unknown_value"),
        Classification::Internal
    );
    assert_eq!(
        Classification::from_str_flexible(""),
        Classification::Internal
    );
}

// ---- CLASSIFY-01j: PATCH classification requires Write role (falsification) ----

#[tokio::test]
async fn classify01j_patch_classification_rejects_read_key() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["rbac-admin-key".to_string()]).await;

    // Create a read-only scoped key
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer rbac-admin-key",
        serde_json::json!({ "name": "reader", "role": "read" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let read_key = create_body["raw_key"].as_str().unwrap().to_string();

    // Try to PATCH classification with read key — should fail (requires Write)
    let (status, _body) = json_request_with_header(
        &app,
        "PATCH",
        &format!("/api/v1/entities/{}/classification", Uuid::from_u128(1)),
        "authorization",
        &format!("Bearer {}", read_key),
        serde_json::json!({ "classification": "public" }),
    )
    .await;
    // NotFound or Forbidden — either is acceptable since the entity doesn't exist,
    // but the important thing is the role check happens first (Forbidden)
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "read key should not be able to PATCH classification"
    );
}

// =============================================================================
// VIEW-01: Memory View CRUD + Policy-Scoped Context Filtering
// =============================================================================

// ---- VIEW-01a: Create a memory view (Admin) ----

#[tokio::test]
async fn view01a_create_view_returns_created() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "support_safe",
            "description": "Safe for customer-facing agents",
            "max_classification": "internal",
            "blocked_edge_labels": ["salary", "ssn"],
            "max_facts": 50,
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CREATED,
        "create view failed: {:?}",
        body
    );
    assert_eq!(body["name"].as_str().unwrap(), "support_safe");
    assert_eq!(body["max_classification"].as_str().unwrap(), "internal");
    assert_eq!(body["max_facts"].as_u64().unwrap(), 50);
    assert!(body["id"].is_string());
    assert!(body["created_at"].is_string());
}

// ---- VIEW-01b: Duplicate view name rejected ----

#[tokio::test]
async fn view01b_duplicate_view_name_rejected() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    let view_body = serde_json::json!({
        "name": "dup_test_view",
        "description": "First",
        "max_classification": "internal",
    });

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        view_body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        view_body,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "duplicate should be rejected: {:?}",
        body
    );
}

// ---- VIEW-01c: List views ----

#[tokio::test]
async fn view01c_list_views() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    // Create two views
    for name in &["list_v1", "list_v2"] {
        let (status, _) = json_request_with_header(
            &app,
            "POST",
            "/api/v1/views",
            "authorization",
            "Bearer view-admin-key",
            serde_json::json!({
                "name": name,
                "description": "test",
                "max_classification": "public",
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, body) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list views failed: {:?}", body);
    let views = body["views"].as_array().unwrap();
    assert!(views.len() >= 2);
}

// ---- VIEW-01d: Get view by name ----

#[tokio::test]
async fn view01d_get_view_by_name() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "get_test_view",
            "description": "For GET test",
            "max_classification": "confidential",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/views/get_test_view",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get view failed: {:?}", body);
    assert_eq!(body["name"].as_str().unwrap(), "get_test_view");
    assert_eq!(body["max_classification"].as_str().unwrap(), "confidential");
}

// ---- VIEW-01e: Update view ----

#[tokio::test]
async fn view01e_update_view() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "update_test",
            "description": "Original",
            "max_classification": "internal",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request_with_header(
        &app,
        "PUT",
        "/api/v1/views/update_test",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "update_test",
            "description": "Updated",
            "max_classification": "public",
            "max_facts": 25,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update view failed: {:?}", body);
    assert_eq!(body["description"].as_str().unwrap(), "Updated");
    assert_eq!(body["max_classification"].as_str().unwrap(), "public");
    assert_eq!(body["max_facts"].as_u64().unwrap(), 25);
}

// ---- VIEW-01f: Delete view ----

#[tokio::test]
async fn view01f_delete_view() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "delete_test",
            "description": "To be deleted",
            "max_classification": "internal",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request_with_header(
        &app,
        "DELETE",
        "/api/v1/views/delete_test",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete view failed");

    // Verify it's gone
    let (status, _) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/views/delete_test",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- VIEW-01g: Read key can list views but cannot create ----

#[tokio::test]
async fn view01g_read_key_can_list_but_not_create() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    // Create a read-only key
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({ "name": "view-reader", "role": "read" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let read_key = create_body["raw_key"].as_str().unwrap().to_string();

    // Listing should succeed (read role)
    let (status, _) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/views",
        "authorization",
        &format!("Bearer {}", read_key),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "read key should be able to list views"
    );

    // Creating should fail (requires admin)
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        &format!("Bearer {}", read_key),
        serde_json::json!({
            "name": "should_fail",
            "description": "nope",
            "max_classification": "internal",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "read key should not create views"
    );
}

// ---- VIEW-01h: Get nonexistent view returns 404 ----

#[tokio::test]
async fn view01h_get_nonexistent_view_returns_404() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    let (status, _) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/views/nonexistent_view_xyz",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- VIEW-02a: Context with view filters entities by classification ----

#[tokio::test]
async fn view02a_context_view_filters_entities_by_classification() {
    let (app, store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    // Create a user
    let user_name = format!("view02a_user_{}", Uuid::now_v7());
    let (status, user_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id: Uuid = user_body["id"].as_str().unwrap().parse().unwrap();

    // Create entities with different classification levels
    let ep_id = Uuid::from_u128(10);
    let public_entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "Coffee Shop".into(),
            entity_type: EntityType::Location,
            summary: Some("A public coffee shop".into()),
            classification: Classification::Public,
        },
        user_id,
        ep_id,
    );
    let public_entity = store.create_entity(public_entity).await.unwrap();

    let confidential_entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "Bank Account".into(),
            entity_type: EntityType::Concept,
            summary: Some("Private bank account".into()),
            classification: Classification::Confidential,
        },
        user_id,
        ep_id,
    );
    let confidential_entity = store.create_entity(confidential_entity).await.unwrap();

    // Create a target entity for edges
    let target_entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "Latte".into(),
            entity_type: EntityType::Concept,
            summary: None,
            classification: Classification::Public,
        },
        user_id,
        ep_id,
    );
    let target_entity = store.create_entity(target_entity).await.unwrap();

    let now = chrono::Utc::now();

    // Create edges from these entities
    let public_edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "Coffee Shop".into(),
            target_name: "Latte".into(),
            label: "serves".into(),
            fact: "Coffee Shop serves Latte".into(),
            confidence: 0.9,
            valid_at: None,
            classification: Classification::Public,
        },
        user_id,
        public_entity.id,
        target_entity.id,
        ep_id,
        now,
    );
    store.create_edge(public_edge).await.unwrap();

    let confidential_edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "Bank Account".into(),
            target_name: "$50,000".into(),
            label: "balance".into(),
            fact: "Bank Account has balance $50,000".into(),
            confidence: 0.9,
            valid_at: None,
            classification: Classification::Confidential,
        },
        user_id,
        confidential_entity.id,
        Uuid::from_u128(101),
        ep_id,
        now,
    );
    store.create_edge(confidential_edge).await.unwrap();

    // Create a view that caps classification at Public
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "public_only_view",
            "description": "Only public data",
            "max_classification": "public",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Request context with the view
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/memory/{}/context", user_name),
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "query": "Tell me about coffee and banking",
            "view": "public_only_view",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "context request failed: {:?}", body);

    // The view_applied field should be set
    assert_eq!(body["view_applied"].as_str(), Some("public_only_view"));

    // Entities with Confidential classification should be filtered out
    let empty_arr = vec![];
    let entities = body["entities"].as_array().unwrap_or(&empty_arr);
    for entity in entities {
        let name = entity["name"].as_str().unwrap_or("");
        assert_ne!(
            name, "Bank Account",
            "Confidential entity should be filtered by public_only view"
        );
    }

    // Facts with Confidential classification should be filtered out
    let facts = body["facts"].as_array().unwrap_or(&empty_arr);
    for fact in facts {
        let label = fact["label"].as_str().unwrap_or("");
        assert_ne!(
            label, "balance",
            "Confidential fact should be filtered by public_only view"
        );
    }
}

// ---- VIEW-02b: Context with view blocks edge labels ----

#[tokio::test]
async fn view02b_context_view_blocks_edge_labels() {
    let (app, store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    let user_name = format!("view02b_user_{}", Uuid::now_v7());
    let (status, user_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id: Uuid = user_body["id"].as_str().unwrap().parse().unwrap();

    let ep_id = Uuid::from_u128(20);
    let entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "Employee".into(),
            entity_type: EntityType::Person,
            summary: Some("An employee".into()),
            classification: Classification::Internal,
        },
        user_id,
        ep_id,
    );
    let entity = store.create_entity(entity).await.unwrap();

    let target1 = Entity::from_extraction(
        &ExtractedEntity {
            name: "SalaryAmount".into(),
            entity_type: EntityType::Concept,
            summary: None,
            classification: Classification::Internal,
        },
        user_id,
        ep_id,
    );
    let target1 = store.create_entity(target1).await.unwrap();
    let target2 = Entity::from_extraction(
        &ExtractedEntity {
            name: "Engineer".into(),
            entity_type: EntityType::Concept,
            summary: None,
            classification: Classification::Internal,
        },
        user_id,
        ep_id,
    );
    let target2 = store.create_entity(target2).await.unwrap();

    let now = chrono::Utc::now();
    let salary_edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "Employee".into(),
            target_name: "SalaryAmount".into(),
            label: "salary".into(),
            fact: "Employee earns $100k salary".into(),
            confidence: 0.9,
            valid_at: None,
            classification: Classification::Internal,
        },
        user_id,
        entity.id,
        target1.id,
        ep_id,
        now,
    );
    store.create_edge(salary_edge).await.unwrap();

    let role_edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "Employee".into(),
            target_name: "Engineer".into(),
            label: "role".into(),
            fact: "Employee is an Engineer".into(),
            confidence: 0.9,
            valid_at: None,
            classification: Classification::Internal,
        },
        user_id,
        entity.id,
        target2.id,
        ep_id,
        now,
    );
    store.create_edge(role_edge).await.unwrap();

    // Create view that blocks salary edges
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "no_salary_view",
            "description": "Hides salary info",
            "max_classification": "restricted",
            "blocked_edge_labels": ["salary"],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/memory/{}/context", user_name),
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "query": "Tell me about the employee",
            "view": "no_salary_view",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "context failed: {:?}", body);

    let empty_arr = vec![];
    let facts = body["facts"].as_array().unwrap_or(&empty_arr);
    for fact in facts {
        let label = fact["label"].as_str().unwrap_or("");
        assert_ne!(label, "salary", "Salary facts should be blocked by view");
    }
}

// ---- VIEW-02c: Context with nonexistent view returns 404 ----

#[tokio::test]
async fn view02c_context_nonexistent_view_returns_404() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    let user_name = format!("view02c_user_{}", Uuid::now_v7());
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/memory/{}/context", user_name),
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "query": "anything",
            "view": "does_not_exist",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "nonexistent view should 404: {:?}",
        body
    );
}

// ---- VIEW-02d: Scoped key classification ceiling narrows view ----

#[tokio::test]
async fn view02d_scoped_key_classification_narrows_view() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    // Create a view that allows Confidential
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "wide_view_02d",
            "description": "Allows up to confidential",
            "max_classification": "confidential",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Create a scoped key with max_classification=internal
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "internal-only-key",
            "role": "read",
            "scope": { "max_classification": "internal" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let scoped_key = create_body["raw_key"].as_str().unwrap().to_string();

    // Create user
    let user_name = format!("view02d_user_{}", Uuid::now_v7());
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Request context with the wide view using the restricted key
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/memory/{}/context", user_name),
        "authorization",
        &format!("Bearer {}", scoped_key),
        serde_json::json!({
            "query": "anything",
            "view": "wide_view_02d",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "context request failed: {:?}", body);
    // The view should be applied — even though the view allows Confidential,
    // the scoped key limits to Internal, so view_applied should be set
    assert_eq!(body["view_applied"].as_str(), Some("wide_view_02d"));
}

// ---- VIEW-02e: View with max_facts caps fact count ----

#[tokio::test]
async fn view02e_view_max_facts_caps_count() {
    let (app, store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    let user_name = format!("view02e_user_{}", Uuid::now_v7());
    let (status, user_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id: Uuid = user_body["id"].as_str().unwrap().parse().unwrap();

    let ep_id = Uuid::from_u128(30);
    let entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "MaxFactsEntity".into(),
            entity_type: EntityType::Concept,
            summary: Some("Test".into()),
            classification: Classification::Public,
        },
        user_id,
        ep_id,
    );
    let entity = store.create_entity(entity).await.unwrap();

    let now = chrono::Utc::now();
    for i in 0u128..10 {
        let target = Entity::from_extraction(
            &ExtractedEntity {
                name: format!("Target{}", i),
                entity_type: EntityType::Concept,
                summary: None,
                classification: Classification::Public,
            },
            user_id,
            ep_id,
        );
        let target = store.create_entity(target).await.unwrap();

        let edge = Edge::from_extraction(
            &ExtractedRelationship {
                source_name: "MaxFactsEntity".into(),
                target_name: format!("Target{}", i),
                label: "related_to".into(),
                fact: format!("MaxFactsEntity is related to Target{}", i),
                confidence: 0.9,
                valid_at: None,
                classification: Classification::Public,
            },
            user_id,
            entity.id,
            target.id,
            ep_id,
            now,
        );
        store.create_edge(edge).await.unwrap();
    }

    // Create view with max_facts=3
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "limited_facts_view",
            "description": "Only 3 facts max",
            "max_classification": "restricted",
            "max_facts": 3,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/memory/{}/context", user_name),
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "query": "Tell me about MaxFactsEntity",
            "view": "limited_facts_view",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "context failed: {:?}", body);

    let empty_arr = vec![];
    let facts = body["facts"].as_array().unwrap_or(&empty_arr);
    assert!(
        facts.len() <= 3,
        "max_facts=3 view should cap facts to at most 3, got {}",
        facts.len()
    );
}

// ---- VIEW-02f: View with include_narrative=false suppresses narrative ----

#[tokio::test]
async fn view02f_view_suppresses_narrative() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    let user_name = format!("view02f_user_{}", Uuid::now_v7());
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Create view with include_narrative=false
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "no_narrative_view",
            "description": "No narrative",
            "max_classification": "restricted",
            "include_narrative": false,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Request context with include_narrative=true at request level, but view says false
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/memory/{}/context", user_name),
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "query": "anything",
            "view": "no_narrative_view",
            "include_narrative": true,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "context failed: {:?}", body);
    // narrative should be null because view suppresses it
    assert!(
        body["narrative"].is_null(),
        "narrative should be suppressed by view"
    );
}

// ---- VIEW-02g: Context without view but with scoped key still filters ----

#[tokio::test]
async fn view02g_no_view_scoped_key_still_filters() {
    let (app, store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    // Create a key with max_classification=public
    let (status, create_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "public-only-key",
            "role": "read",
            "scope": { "max_classification": "public" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let public_key = create_body["raw_key"].as_str().unwrap().to_string();

    let user_name = format!("view02g_user_{}", Uuid::now_v7());
    let (status, user_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id: Uuid = user_body["id"].as_str().unwrap().parse().unwrap();

    let ep_id = Uuid::from_u128(40);
    // Create an entity with Internal classification
    let internal_entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "InternalProject".into(),
            entity_type: EntityType::Concept,
            summary: Some("Internal project".into()),
            classification: Classification::Internal,
        },
        user_id,
        ep_id,
    );
    let internal_entity = store.create_entity(internal_entity).await.unwrap();

    let now = chrono::Utc::now();
    let target = Entity::from_extraction(
        &ExtractedEntity {
            name: "Roadmap".into(),
            entity_type: EntityType::Concept,
            summary: None,
            classification: Classification::Internal,
        },
        user_id,
        ep_id,
    );
    let target = store.create_entity(target).await.unwrap();

    // Create edge with Internal classification
    let internal_edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "InternalProject".into(),
            target_name: "Roadmap".into(),
            label: "has".into(),
            fact: "InternalProject has Roadmap".into(),
            confidence: 0.9,
            valid_at: None,
            classification: Classification::Internal,
        },
        user_id,
        internal_entity.id,
        target.id,
        ep_id,
        now,
    );
    store.create_edge(internal_edge).await.unwrap();

    // Request context without view, using the public-only key
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/memory/{}/context", user_name),
        "authorization",
        &format!("Bearer {}", public_key),
        serde_json::json!({ "query": "Tell me about InternalProject" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "context failed: {:?}", body);

    // No view applied (no explicit view), but scoped key filtering is active
    assert!(
        body["view_applied"].is_null(),
        "no explicit view was applied"
    );

    // Internal entities/facts should be filtered out by the public-only key
    let empty_arr = vec![];
    let entities = body["entities"].as_array().unwrap_or(&empty_arr);
    for entity in entities {
        let name = entity["name"].as_str().unwrap_or("");
        assert_ne!(
            name, "InternalProject",
            "Internal entity should be filtered by public-only key"
        );
    }
}

// ---- VIEW-02h: View name validation ----

#[tokio::test]
async fn view02h_view_name_validation() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["view-admin-key".to_string()]).await;

    // Empty name
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": "  ",
            "description": "Bad name",
            "max_classification": "internal",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "empty name should be rejected"
    );

    // Very long name (>128 chars)
    let long_name = "a".repeat(200);
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/views",
        "authorization",
        "Bearer view-admin-key",
        serde_json::json!({
            "name": long_name,
            "description": "Too long",
            "max_classification": "internal",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "long name should be rejected"
    );
}

// =============================================================================
// GR-01: Guardrail CRUD
// =============================================================================

// ---- GR-01a: Create a guardrail rule (Admin) ----

#[tokio::test]
async fn gr01a_create_guardrail_returns_created() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "block_ssn_storage",
            "description": "Block episodes containing SSN references",
            "trigger": "on_ingest",
            "condition": { "type": "content_matches_regex", "pattern": "\\bSSN\\b" },
            "action": { "type": "block", "reason": "SSN references not allowed" },
            "priority": 10,
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CREATED,
        "create guardrail failed: {:?}",
        body
    );
    assert_eq!(body["name"].as_str().unwrap(), "block_ssn_storage");
    assert_eq!(body["priority"].as_u64().unwrap(), 10);
    assert!(body["enabled"].as_bool().unwrap());
    assert!(body["id"].is_string());
    assert!(body["created_at"].is_string());
}

// ---- GR-01b: Duplicate guardrail name rejected ----

#[tokio::test]
async fn gr01b_duplicate_guardrail_name_rejected() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    let body = serde_json::json!({
        "name": "dup_guard",
        "description": "First",
        "trigger": "on_any",
        "condition": { "type": "classification_above", "classification": "internal" },
        "action": { "type": "audit_only", "severity": "info" },
    });

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body2) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        body,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "duplicate should be rejected: {:?}",
        body2
    );
}

// ---- GR-01c: List guardrails ----

#[tokio::test]
async fn gr01c_list_guardrails() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    // Create two rules
    for name in &["gr_list_a", "gr_list_b"] {
        let (status, _) = json_request_with_header(
            &app,
            "POST",
            "/api/v1/guardrails",
            "authorization",
            "Bearer gr-admin-key",
            serde_json::json!({
                "name": name,
                "description": "test rule",
                "trigger": "on_retrieval",
                "condition": { "type": "classification_above", "classification": "public" },
                "action": { "type": "redact" },
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, body) = json_request_with_header(
        &app,
        "GET",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list failed: {:?}", body);
    let rules = body["guardrails"].as_array().unwrap();
    assert!(rules.len() >= 2);
}

// ---- GR-01d: Get guardrail by ID ----

#[tokio::test]
async fn gr01d_get_guardrail_by_id() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "gr_get_test",
            "description": "Get test",
            "trigger": "on_ingest",
            "condition": { "type": "edge_label_in", "labels": ["salary"] },
            "action": { "type": "block", "reason": "no salary" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();

    let (status, body) = json_request_with_header(
        &app,
        "GET",
        &format!("/api/v1/guardrails/{}", id),
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get failed: {:?}", body);
    assert_eq!(body["name"].as_str().unwrap(), "gr_get_test");
}

// ---- GR-01e: Update guardrail ----

#[tokio::test]
async fn gr01e_update_guardrail() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "gr_update_test",
            "description": "Before update",
            "trigger": "on_ingest",
            "condition": { "type": "classification_above", "classification": "internal" },
            "action": { "type": "audit_only", "severity": "low" },
            "priority": 50,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();

    let (status, body) = json_request_with_header(
        &app,
        "PUT",
        &format!("/api/v1/guardrails/{}", id),
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "gr_update_test",
            "description": "After update",
            "trigger": "on_retrieval",
            "condition": { "type": "classification_above", "classification": "confidential" },
            "action": { "type": "block", "reason": "upgraded to block" },
            "priority": 5,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {:?}", body);
    assert_eq!(body["description"].as_str().unwrap(), "After update");
    assert_eq!(body["priority"].as_u64().unwrap(), 5);
}

// ---- GR-01f: Delete guardrail ----

#[tokio::test]
async fn gr01f_delete_guardrail() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "gr_delete_test",
            "description": "Will be deleted",
            "trigger": "on_any",
            "condition": { "type": "confidence_below", "confidence": 0.5 },
            "action": { "type": "warn", "message": "low confidence" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();

    let (status, _) = json_request_with_header(
        &app,
        "DELETE",
        &format!("/api/v1/guardrails/{}", id),
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify it's gone
    let (status, _) = json_request_with_header(
        &app,
        "GET",
        &format!("/api/v1/guardrails/{}", id),
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- GR-01g: RBAC — read key cannot create guardrails ----

#[tokio::test]
async fn gr01g_read_key_cannot_create_guardrails() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    // Create a read-only scoped key
    let (status, key_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "name": "gr-reader", "role": "read" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let read_key = key_body["raw_key"].as_str().unwrap().to_string();

    // Try to create a guardrail with the read key — should be forbidden
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        &format!("Bearer {}", read_key),
        serde_json::json!({
            "name": "should_fail",
            "description": "Read cannot create",
            "trigger": "on_any",
            "condition": { "type": "classification_above", "classification": "public" },
            "action": { "type": "block", "reason": "nope" },
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "read key should not create guardrails"
    );
}

// ---- GR-01h: Invalid regex rejected ----

#[tokio::test]
async fn gr01h_invalid_regex_rejected() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "bad_regex",
            "description": "Invalid regex pattern",
            "trigger": "on_ingest",
            "condition": { "type": "content_matches_regex", "pattern": "[invalid" },
            "action": { "type": "block", "reason": "bad pattern" },
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "invalid regex should be rejected: {:?}",
        body
    );
}

// ---- GR-01i: Get nonexistent guardrail returns 404 ----

#[tokio::test]
async fn gr01i_nonexistent_guardrail_returns_404() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    let fake_id = Uuid::from_u128(999999);
    let (status, _) = json_request_with_header(
        &app,
        "GET",
        &format!("/api/v1/guardrails/{}", fake_id),
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// =============================================================================
// GR-02: Guardrail Write-Path Enforcement (Episode Ingestion)
// =============================================================================

// ---- GR-02a: Block rule prevents episode storage ----

#[tokio::test]
async fn gr02a_block_rule_prevents_episode_storage() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    // Create user + session
    let user_name = format!("gr02a_user_{}", Uuid::now_v7());
    let (status, user_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, session_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/sessions",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "user_id": user_body["id"], "name": "test-session" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session_body["id"].as_str().unwrap();

    // Create a guardrail that blocks SSN content
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "block_ssn_ingest",
            "description": "Block SSN references during ingestion",
            "trigger": "on_ingest",
            "condition": { "type": "content_matches_regex", "pattern": "\\bSSN\\b" },
            "action": { "type": "block", "reason": "SSN data not allowed in memory" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Try to ingest an episode with SSN content — should be blocked
    let ep_url = format!("/api/v1/sessions/{}/episodes", session_id);
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        &ep_url,
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "type": "message",
            "content": "My SSN is 123-45-6789, please remember it.",
            "role": "user",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "SSN content should be blocked: {:?}",
        body
    );
    let msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("Guardrail blocked"),
        "Error should mention guardrail: {}",
        msg
    );

    // Ingest an episode WITHOUT SSN — should succeed
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        &ep_url,
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "type": "message",
            "content": "I like coffee.",
            "role": "user",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "safe content should be accepted"
    );
}

// =============================================================================
// GR-03: Guardrail Read-Path Enforcement (Context Retrieval)
// =============================================================================

// ---- GR-03a: Redact rule removes matching facts from context ----

#[tokio::test]
async fn gr03a_redact_rule_removes_matching_facts() {
    let (app, store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    use mnemo_core::models::classification::Classification;

    let user_name = format!("gr03a_user_{}", Uuid::now_v7());
    let (status, user_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id: Uuid = user_body["id"].as_str().unwrap().parse().unwrap();

    // Create entities + edges (one salary edge, one normal edge)
    let ep_id = Uuid::from_u128(300);
    let person = Entity::from_extraction(
        &ExtractedEntity {
            name: "Alice".into(),
            entity_type: EntityType::Person,
            summary: Some("An employee".into()),
            classification: Classification::Internal,
        },
        user_id,
        ep_id,
    );
    let person = store.create_entity(person).await.unwrap();

    let company = Entity::from_extraction(
        &ExtractedEntity {
            name: "Acme Corp".into(),
            entity_type: EntityType::Organization,
            summary: Some("A company".into()),
            classification: Classification::Internal,
        },
        user_id,
        ep_id,
    );
    let company = store.create_entity(company).await.unwrap();

    let now = chrono::Utc::now();
    let salary_edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "Alice".into(),
            target_name: "Acme Corp".into(),
            label: "salary_at".into(),
            fact: "Alice earns $150k at Acme Corp".into(),
            confidence: 0.9,
            valid_at: None,
            classification: Classification::Internal,
        },
        user_id,
        person.id,
        company.id,
        ep_id,
        now,
    );
    store.create_edge(salary_edge).await.unwrap();

    let works_edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "Alice".into(),
            target_name: "Acme Corp".into(),
            label: "works_at".into(),
            fact: "Alice works at Acme Corp".into(),
            confidence: 0.9,
            valid_at: None,
            classification: Classification::Internal,
        },
        user_id,
        person.id,
        company.id,
        ep_id,
        now,
    );
    store.create_edge(works_edge).await.unwrap();

    // Create a guardrail that redacts salary-labeled edges on retrieval
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "redact_salary",
            "description": "Redact salary facts from context",
            "trigger": "on_retrieval",
            "condition": { "type": "edge_label_in", "labels": ["salary_at"] },
            "action": { "type": "redact" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Request context
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/memory/{}/context", user_name),
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "query": "Tell me about Alice" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "context failed: {:?}", body);

    // Salary facts should be redacted
    let empty_arr = vec![];
    let facts = body["facts"].as_array().unwrap_or(&empty_arr);
    for fact in facts {
        let label = fact["label"].as_str().unwrap_or("");
        assert_ne!(
            label, "salary_at",
            "Salary fact should be redacted by guardrail"
        );
    }
}

// ---- GR-03b: Warn rule adds warnings to context response ----

#[tokio::test]
async fn gr03b_warn_rule_adds_warnings_to_response() {
    let (app, store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    use mnemo_core::models::classification::Classification;

    let user_name = format!("gr03b_user_{}", Uuid::now_v7());
    let (status, user_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id: Uuid = user_body["id"].as_str().unwrap().parse().unwrap();

    // Create an entity + edge with confidential classification
    let ep_id = Uuid::from_u128(301);
    let entity = Entity::from_extraction(
        &ExtractedEntity {
            name: "HR Records".into(),
            entity_type: EntityType::Concept,
            summary: Some("HR data".into()),
            classification: Classification::Confidential,
        },
        user_id,
        ep_id,
    );
    let entity = store.create_entity(entity).await.unwrap();
    let target = Entity::from_extraction(
        &ExtractedEntity {
            name: "Sensitive Data".into(),
            entity_type: EntityType::Concept,
            summary: None,
            classification: Classification::Confidential,
        },
        user_id,
        ep_id,
    );
    let target = store.create_entity(target).await.unwrap();

    let now = chrono::Utc::now();
    let edge = Edge::from_extraction(
        &ExtractedRelationship {
            source_name: "HR Records".into(),
            target_name: "Sensitive Data".into(),
            label: "contains".into(),
            fact: "HR Records contains Sensitive Data".into(),
            confidence: 0.9,
            valid_at: None,
            classification: Classification::Confidential,
        },
        user_id,
        entity.id,
        target.id,
        ep_id,
        now,
    );
    store.create_edge(edge).await.unwrap();

    // Create a guardrail that warns on confidential data access
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "warn_confidential_access",
            "description": "Warn when accessing confidential data",
            "trigger": "on_retrieval",
            "condition": { "type": "classification_above", "classification": "internal" },
            "action": { "type": "warn", "message": "Accessing confidential data" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Request context
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/memory/{}/context", user_name),
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "query": "Tell me about HR records" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "context failed: {:?}", body);

    // Check for warnings in response
    let warnings = body["guardrail_warnings"].as_array();
    assert!(
        warnings.is_some(),
        "guardrail_warnings should be present: {:?}",
        body
    );
    let warnings = warnings.unwrap();
    assert!(!warnings.is_empty(), "Should have at least one warning");
    let first_warning = warnings[0].as_str().unwrap();
    assert!(
        first_warning.contains("confidential"),
        "Warning should mention confidential: {}",
        first_warning
    );
}

// =============================================================================
// GR-04: Dry-Run Evaluate Endpoint
// =============================================================================

// ---- GR-04a: Dry-run returns rule evaluation results ----

#[tokio::test]
async fn gr04a_dryrun_evaluate_returns_results() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    // Create a block rule
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "dryrun_block_test",
            "description": "Block test for dry-run",
            "trigger": "on_ingest",
            "condition": { "type": "content_matches_regex", "pattern": "\\bpassword\\b" },
            "action": { "type": "block", "reason": "password content blocked" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Dry-run with matching content
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails/evaluate",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "trigger": "on_ingest",
            "content": "My password is secret123",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "evaluate failed: {:?}", body);
    assert!(body["blocked"].as_bool().unwrap(), "Should be blocked");
    assert_eq!(
        body["block_reason"].as_str().unwrap(),
        "password content blocked"
    );

    // Dry-run with non-matching content
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails/evaluate",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "trigger": "on_ingest",
            "content": "I like coffee",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "evaluate failed: {:?}", body);
    assert!(!body["blocked"].as_bool().unwrap(), "Should not be blocked");
}

// =============================================================================
// GR-05: Condition Combinators
// =============================================================================

// ---- GR-05a: AND combinator blocks only when all conditions match ----

#[tokio::test]
async fn gr05a_and_combinator_blocks_when_all_match() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    // Create user + session
    let user_name = format!("gr05a_user_{}", Uuid::now_v7());
    let (status, user_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, session_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/sessions",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "user_id": user_body["id"], "name": "and-test" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session_body["id"].as_str().unwrap();

    // Create rule: block only if content matches BOTH "credit" AND "card"
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "and_combo_test",
            "description": "Block credit card data",
            "trigger": "on_ingest",
            "condition": {
                "type": "and",
                "conditions": [
                    { "type": "content_matches_regex", "pattern": "\\bcredit\\b" },
                    { "type": "content_matches_regex", "pattern": "\\bcard\\b" },
                ]
            },
            "action": { "type": "block", "reason": "credit card data not allowed" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // "credit card" → blocked
    let (status, _) = json_request_with_header(
        &app, "POST", &format!("/api/v1/sessions/{}/episodes", session_id),
        "authorization", "Bearer gr-admin-key",
        serde_json::json!({ "type": "message", "content": "My credit card number is 4111", "role": "user" }),
    ).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "credit card should be blocked"
    );

    // "credit score" → NOT blocked (only matches one condition)
    let (status, _) = json_request_with_header(
        &app, "POST", &format!("/api/v1/sessions/{}/episodes", session_id),
        "authorization", "Bearer gr-admin-key",
        serde_json::json!({ "type": "message", "content": "My credit score is 750", "role": "user" }),
    ).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "credit score should pass AND combinator"
    );
}

// ---- GR-05b: OR combinator blocks when any condition matches ----

#[tokio::test]
async fn gr05b_or_combinator_blocks_when_any_matches() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    let user_name = format!("gr05b_user_{}", Uuid::now_v7());
    let (status, user_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "name": user_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, session_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/sessions",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "user_id": user_body["id"], "name": "or-test" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = session_body["id"].as_str().unwrap();

    // Create rule: block if content matches SSN OR passport
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "or_combo_test",
            "description": "Block any ID documents",
            "trigger": "on_ingest",
            "condition": {
                "type": "or",
                "conditions": [
                    { "type": "content_matches_regex", "pattern": "\\bSSN\\b" },
                    { "type": "content_matches_regex", "pattern": "\\bpassport\\b" },
                ]
            },
            "action": { "type": "block", "reason": "ID document data not allowed" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // SSN → blocked
    let (status, _) = json_request_with_header(
        &app, "POST", &format!("/api/v1/sessions/{}/episodes", session_id),
        "authorization", "Bearer gr-admin-key",
        serde_json::json!({ "type": "message", "content": "My SSN is 123-45-6789", "role": "user" }),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "SSN should be blocked");

    // passport → blocked
    let (status, _) = json_request_with_header(
        &app, "POST", &format!("/api/v1/sessions/{}/episodes", session_id),
        "authorization", "Bearer gr-admin-key",
        serde_json::json!({ "type": "message", "content": "My passport number is AB123456", "role": "user" }),
    ).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "passport should be blocked"
    );

    // safe content → not blocked
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/sessions/{}/episodes", session_id),
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "type": "message", "content": "I like hiking", "role": "user" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "safe content should pass");
}

// ---- GR-05c: NOT combinator inverts condition ----

#[tokio::test]
async fn gr05c_not_combinator_inverts_condition() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    // Create a rule that blocks everything EXCEPT content matching "approved"
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "not_combo_test",
            "description": "Block anything not approved",
            "trigger": "on_ingest",
            "condition": {
                "type": "not",
                "condition": { "type": "content_matches_regex", "pattern": "\\bapproved\\b" }
            },
            "action": { "type": "block", "reason": "Only approved content allowed" },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Dry-run: "this is approved content" → NOT blocked
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails/evaluate",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "trigger": "on_ingest",
            "content": "This is approved content",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body["blocked"].as_bool().unwrap(),
        "Approved content should not be blocked"
    );

    // Dry-run: "random text" → blocked
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails/evaluate",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "trigger": "on_ingest",
            "content": "random text without keyword",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["blocked"].as_bool().unwrap(),
        "Unapproved content should be blocked"
    );
}

// =============================================================================
// GR-06: Priority Ordering + Disabled Rules
// =============================================================================

// ---- GR-06a: Lower priority rule fires first ----

#[tokio::test]
async fn gr06a_lower_priority_fires_first() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    // Create two rules with different priorities
    // Priority 5 → block with "first"
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "priority_high",
            "description": "Higher priority",
            "trigger": "on_ingest",
            "condition": { "type": "content_matches_regex", "pattern": "\\btest\\b" },
            "action": { "type": "block", "reason": "blocked by priority 5" },
            "priority": 5,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Priority 20 → block with "second"
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "priority_low",
            "description": "Lower priority",
            "trigger": "on_ingest",
            "condition": { "type": "content_matches_regex", "pattern": "\\btest\\b" },
            "action": { "type": "block", "reason": "blocked by priority 20" },
            "priority": 20,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Dry-run: the block reason should come from the priority 5 rule
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails/evaluate",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "trigger": "on_ingest",
            "content": "this is a test",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["blocked"].as_bool().unwrap());
    assert_eq!(
        body["block_reason"].as_str().unwrap(),
        "blocked by priority 5"
    );
}

// ---- GR-06b: Disabled rule does not fire ----

#[tokio::test]
async fn gr06b_disabled_rule_does_not_fire() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    // Create a disabled block rule
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "disabled_block",
            "description": "Disabled rule",
            "trigger": "on_ingest",
            "condition": { "type": "content_matches_regex", "pattern": "\\beverything\\b" },
            "action": { "type": "block", "reason": "should not fire" },
            "enabled": false,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Dry-run: should NOT be blocked
    let (status, body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails/evaluate",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "trigger": "on_ingest",
            "content": "everything is fine",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !body["blocked"].as_bool().unwrap(),
        "Disabled rule should not block"
    );
}

// =============================================================================
// GR-07: Scope Isolation
// =============================================================================

// ---- GR-07a: User-scoped rule only applies to that user ----

#[tokio::test]
async fn gr07a_user_scoped_rule_only_applies_to_that_user() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["gr-admin-key".to_string()]).await;

    // Create two users
    let user1_name = format!("gr07a_user1_{}", Uuid::now_v7());
    let (status, user1_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "name": user1_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user1_id = user1_body["id"].as_str().unwrap();

    let user2_name = format!("gr07a_user2_{}", Uuid::now_v7());
    let (status, user2_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "name": user2_name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user2_id = user2_body["id"].as_str().unwrap();

    // Create sessions for both
    let (status, sess1_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/sessions",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "user_id": user1_id, "name": "sess1" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let sess1_id = sess1_body["id"].as_str().unwrap();

    let (status, sess2_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/sessions",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({ "user_id": user2_id, "name": "sess2" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let sess2_id = sess2_body["id"].as_str().unwrap();

    // Create a user-scoped guardrail: block "forbidden" only for user1
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/guardrails",
        "authorization",
        "Bearer gr-admin-key",
        serde_json::json!({
            "name": "user1_block",
            "description": "Block forbidden for user1 only",
            "trigger": "on_ingest",
            "condition": { "type": "content_matches_regex", "pattern": "\\bforbidden\\b" },
            "action": { "type": "block", "reason": "forbidden for user1" },
            "scope": { "type": "user", "user_id": user1_id },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // User1: "forbidden content" → blocked
    let (status, _) = json_request_with_header(
        &app, "POST", &format!("/api/v1/sessions/{}/episodes", sess1_id),
        "authorization", "Bearer gr-admin-key",
        serde_json::json!({ "type": "message", "content": "This is forbidden content", "role": "user" }),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "user1 should be blocked");

    // User2: "forbidden content" → NOT blocked (rule is user1-scoped)
    let (status, _) = json_request_with_header(
        &app, "POST", &format!("/api/v1/sessions/{}/episodes", sess2_id),
        "authorization", "Bearer gr-admin-key",
        serde_json::json!({ "type": "message", "content": "This is forbidden content", "role": "user" }),
    ).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "user2 should not be blocked by user1-scoped rule"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Feature 5: Agent Identity Phase B — Governance & Conflict Handling
// ═══════════════════════════════════════════════════════════════════

/// Helper: create 3 experience events for an agent and return their IDs.
async fn create_three_experience_events(app: &axum::Router, agent_id: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for (i, signal) in ["signal alpha", "signal beta", "signal gamma"]
        .iter()
        .enumerate()
    {
        let (status, body) = json_request(
            app,
            "POST",
            &format!("/api/v1/agents/{agent_id}/experience"),
            serde_json::json!({
                "category": "tone",
                "signal": signal,
                "confidence": 0.8 + (i as f64) * 0.05,
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "experience event {i} failed: {body:?}"
        );
        ids.push(body["id"].as_str().unwrap().to_string());
    }
    ids
}

// F5-INT-01: Default approval policy returns 1 approver for all risk levels
#[tokio::test]
async fn f5_default_approval_policy_returns_defaults() {
    let app = build_test_app().await;
    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/agents/f5-policy-agent/approval-policy",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "get default policy failed: {body:?}"
    );
    assert_eq!(body["agent_id"], "f5-policy-agent");
    assert_eq!(body["low_risk"]["min_approvers"], 1);
    assert_eq!(body["medium_risk"]["min_approvers"], 1);
    assert_eq!(body["high_risk"]["min_approvers"], 1);
}

// F5-INT-02: Set approval policy requires Admin role
#[tokio::test]
async fn f5_set_approval_policy_requires_admin() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["f5-admin-key".to_string()]).await;

    // Create a read-only key
    let (status, key_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer f5-admin-key",
        serde_json::json!({
            "name": "f5-read-key",
            "role": "read",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let read_key = key_body["raw_key"].as_str().unwrap().to_string();

    // Read key should be forbidden from setting approval policy
    let (status, body) = json_request_with_header(
        &app,
        "PUT",
        "/api/v1/agents/f5-rbac-agent/approval-policy",
        "authorization",
        &format!("Bearer {read_key}"),
        serde_json::json!({
            "low_risk": {"min_approvers": 1},
            "medium_risk": {"min_approvers": 2},
            "high_risk": {"min_approvers": 3},
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "read key should be forbidden: {body:?}"
    );

    // Admin key should succeed
    let (status, body) = json_request_with_header(
        &app,
        "PUT",
        "/api/v1/agents/f5-rbac-agent/approval-policy",
        "authorization",
        "Bearer f5-admin-key",
        serde_json::json!({
            "low_risk": {"min_approvers": 1},
            "medium_risk": {"min_approvers": 2},
            "high_risk": {"min_approvers": 3, "cooling_period_hours": 24},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin set policy failed: {body:?}");
    assert_eq!(body["high_risk"]["min_approvers"], 3);
    assert_eq!(body["high_risk"]["cooling_period_hours"], 24);
}

// F5-INT-03: Set and retrieve custom approval policy
#[tokio::test]
async fn f5_set_and_get_approval_policy() {
    let app = build_test_app().await;

    // Set a custom policy
    let (status, _) = json_request(
        &app,
        "PUT",
        "/api/v1/agents/f5-custom-policy/approval-policy",
        serde_json::json!({
            "low_risk": {"min_approvers": 1},
            "medium_risk": {"min_approvers": 2, "cooling_period_hours": 6},
            "high_risk": {"min_approvers": 3, "cooling_period_hours": 24, "auto_reject_after_hours": 72},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Retrieve it
    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/agents/f5-custom-policy/approval-policy",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["medium_risk"]["min_approvers"], 2);
    assert_eq!(body["medium_risk"]["cooling_period_hours"], 6);
    assert_eq!(body["high_risk"]["auto_reject_after_hours"], 72);
}

// F5-INT-04: Single-approver (default policy) approve flow works as before
#[tokio::test]
async fn f5_single_approver_default_approve_flow() {
    let app = build_test_app().await;
    let agent = "f5-single-approve";
    let event_ids = create_three_experience_events(&app, agent).await;

    let (status, proposal) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions"),
        serde_json::json!({
            "proposal": "adopt formal tone",
            "candidate_core": {"style": "formal"},
            "reason": "evidence supports it",
            "risk_level": "low",
            "source_event_ids": event_ids,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proposal["id"].as_str().unwrap();

    // Single approval should be enough (default policy: 1 approver)
    let (status, approved) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "approve failed: {approved:?}");
    assert_eq!(approved["status"], "approved");
    assert!(approved["approved_at"].is_string());
    assert!(!approved["approvers"].as_array().unwrap().is_empty());
}

// F5-INT-05: Multi-approver quorum — high-risk needs 3, single approval keeps pending
#[tokio::test]
async fn f5_multi_approver_quorum_high_risk() {
    let app = build_test_app().await;
    let agent = "f5-multi-quorum";

    // Set policy: high_risk needs 3 approvers
    let (status, _) = json_request(
        &app,
        "PUT",
        &format!("/api/v1/agents/{agent}/approval-policy"),
        serde_json::json!({
            "low_risk": {"min_approvers": 1},
            "medium_risk": {"min_approvers": 2},
            "high_risk": {"min_approvers": 3},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let event_ids = create_three_experience_events(&app, agent).await;

    let (status, proposal) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions"),
        serde_json::json!({
            "proposal": "major personality shift",
            "candidate_core": {"persona": "bold leader"},
            "reason": "evidence supports it",
            "risk_level": "high",
            "source_event_ids": event_ids,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proposal["id"].as_str().unwrap();

    // First approval — still pending (need 3, have 1)
    // Note: in unauthed mode, caller is always "bootstrap", so repeated
    // approvals from the same caller won't increase the count.
    // We test the basic flow: first call keeps pending because bootstrap is
    // deduplicated. But since we have only one caller identity in unauthed mode,
    // the quorum check is: 1 approver < 3 required → stays pending.
    let (status, partial) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "partial approve failed: {partial:?}"
    );
    assert_eq!(
        partial["status"], "pending",
        "should stay pending with 1/3 approvers"
    );
    assert_eq!(partial["approvers"].as_array().unwrap().len(), 1);
}

// F5-INT-06: Multi-approver quorum with authed app — 3 different keys
#[tokio::test]
async fn f5_multi_approver_quorum_with_authed_keys() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["f5-quorum-admin".to_string()]).await;

    let agent = "f5-quorum-authed";
    let bearer = |k: &str| format!("Bearer {k}");

    // Set policy: high_risk needs 3 approvers
    let (status, _) = json_request_with_header(
        &app,
        "PUT",
        &format!("/api/v1/agents/{agent}/approval-policy"),
        "authorization",
        &bearer("f5-quorum-admin"),
        serde_json::json!({
            "low_risk": {"min_approvers": 1},
            "medium_risk": {"min_approvers": 2},
            "high_risk": {"min_approvers": 3},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Create 3 admin keys
    let mut keys = Vec::new();
    for i in 1..=3 {
        let (status, kb) = json_request_with_header(
            &app,
            "POST",
            "/api/v1/keys",
            "authorization",
            &bearer("f5-quorum-admin"),
            serde_json::json!({ "name": format!("approver-{i}"), "role": "admin" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        keys.push(kb["raw_key"].as_str().unwrap().to_string());
    }

    // Create experience events
    let mut event_ids = Vec::new();
    for sig in ["a signal", "b signal", "c signal"].iter() {
        let (status, ev) = json_request_with_header(
            &app,
            "POST",
            &format!("/api/v1/agents/{agent}/experience"),
            "authorization",
            &bearer("f5-quorum-admin"),
            serde_json::json!({"category":"tone","signal":sig,"confidence":0.8}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        event_ids.push(ev["id"].as_str().unwrap().to_string());
    }

    // Create high-risk proposal
    let (status, proposal) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions"),
        "authorization",
        &bearer("f5-quorum-admin"),
        serde_json::json!({
            "proposal": "major change",
            "candidate_core": {"persona": "bold"},
            "reason": "evidence",
            "risk_level": "high",
            "source_event_ids": event_ids,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proposal["id"].as_str().unwrap();

    // Approve with key 1 → partial (1/3)
    let (status, p1) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/approve"),
        "authorization",
        &bearer(&keys[0]),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(p1["status"], "pending", "1/3 approvals → pending");

    // Approve with key 2 → partial (2/3)
    let (status, p2) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/approve"),
        "authorization",
        &bearer(&keys[1]),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(p2["status"], "pending", "2/3 approvals → pending");

    // Approve with key 3 → approved (3/3)
    let (status, p3) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/approve"),
        "authorization",
        &bearer(&keys[2]),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "final approve failed: {p3:?}");
    assert_eq!(p3["status"], "approved", "3/3 approvals → approved");
    assert_eq!(p3["approvers"].as_array().unwrap().len(), 3);

    // Verify identity was updated
    let (status, identity) = json_request_with_header(
        &app,
        "GET",
        &format!("/api/v1/agents/{agent}/identity"),
        "authorization",
        &bearer("f5-quorum-admin"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(identity["core"]["persona"], "bold");
}

// F5-INT-07: Conflict analysis endpoint returns analysis
#[tokio::test]
async fn f5_conflict_analysis_endpoint() {
    let app = build_test_app().await;
    let agent = "f5-conflict-agent";

    // Add supporting experience
    let (_, ev1) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/experience"),
        serde_json::json!({"category":"tone","signal":"formal style works well","confidence":0.9}),
    )
    .await;
    let (_, ev2) = json_request(
        &app, "POST", &format!("/api/v1/agents/{agent}/experience"),
        serde_json::json!({"category":"tone","signal":"formal approach preferred","confidence":0.85}),
    ).await;
    // Add opposing experience
    let (_, ev3) = json_request(
        &app, "POST", &format!("/api/v1/agents/{agent}/experience"),
        serde_json::json!({"category":"tone","signal":"avoid formal style entirely","confidence":0.7}),
    ).await;

    let event_ids: Vec<String> = vec![
        ev1["id"].as_str().unwrap().into(),
        ev2["id"].as_str().unwrap().into(),
        ev3["id"].as_str().unwrap().into(),
    ];

    // Create proposal about formal style
    let (status, proposal) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions"),
        serde_json::json!({
            "proposal": "adopt formal communication style",
            "candidate_core": {"style": "formal communication"},
            "reason": "evidence",
            "source_event_ids": event_ids,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proposal["id"].as_str().unwrap();

    // Get conflict analysis
    let (status, analysis) = json_request(
        &app,
        "GET",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/conflicts"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "conflict analysis failed: {analysis:?}"
    );
    assert_eq!(analysis["agent_id"], agent);
    assert_eq!(analysis["proposal_id"], pid);
    assert!(analysis["conflict_score"].is_number());
    assert!(analysis["recommendation"].is_string());
    assert!(analysis["supporting_signals"].is_array());
    assert!(analysis["conflicting_signals"].is_array());
}

// F5-INT-08: Reject promotion adds governance audit
#[tokio::test]
async fn f5_reject_promotion_records_audit() {
    let app = build_test_app().await;
    let agent = "f5-reject-audit";
    let event_ids = create_three_experience_events(&app, agent).await;

    let (status, proposal) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions"),
        serde_json::json!({
            "proposal": "test proposal",
            "candidate_core": {"mission": "new"},
            "reason": "test",
            "source_event_ids": event_ids,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proposal["id"].as_str().unwrap();

    let (status, rejected) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/reject"),
        serde_json::json!({"reason": "not convinced"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "reject failed: {rejected:?}");
    assert_eq!(rejected["status"], "rejected");
    assert!(rejected["rejected_at"].is_string());
    assert!(rejected["reason"]
        .as_str()
        .unwrap()
        .contains("not convinced"));
}

// F5-INT-09: Duplicate approver is deduplicated
#[tokio::test]
async fn f5_duplicate_approver_deduplicated() {
    let app = build_test_app().await;
    let agent = "f5-dedup-approver";

    // Set policy: medium needs 2 approvers
    let (status, _) = json_request(
        &app,
        "PUT",
        &format!("/api/v1/agents/{agent}/approval-policy"),
        serde_json::json!({
            "low_risk": {"min_approvers": 1},
            "medium_risk": {"min_approvers": 2},
            "high_risk": {"min_approvers": 3},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let event_ids = create_three_experience_events(&app, agent).await;

    let (status, proposal) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions"),
        serde_json::json!({
            "proposal": "test",
            "candidate_core": {"mission": "new"},
            "reason": "test",
            "source_event_ids": event_ids,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proposal["id"].as_str().unwrap();

    // Approve twice with the same caller (bootstrap) — should remain at 1 approver
    let (status, p1) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(p1["approvers"].as_array().unwrap().len(), 1);

    let (status, p2) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Still 1 approver (deduplicated)
    assert_eq!(p2["approvers"].as_array().unwrap().len(), 1);
    assert_eq!(
        p2["status"], "pending",
        "still pending — need 2, have 1 unique"
    );
}

// F5-INT-10: Approving an already-approved proposal fails
#[tokio::test]
async fn f5_approve_already_approved_fails() {
    let app = build_test_app().await;
    let agent = "f5-double-approve";
    let event_ids = create_three_experience_events(&app, agent).await;

    let (status, proposal) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions"),
        serde_json::json!({
            "proposal": "test",
            "candidate_core": {"mission": "new"},
            "reason": "test",
            "source_event_ids": event_ids,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proposal["id"].as_str().unwrap();

    // Approve
    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Second approval attempt should fail
    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/approve"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "double approve should fail: {body:?}"
    );
}

// F5-INT-11: Expired status serde in list promotions
#[tokio::test]
async fn f5_expired_status_visible_in_list() {
    let app = build_test_app().await;
    let agent = "f5-expired-list";
    let event_ids = create_three_experience_events(&app, agent).await;

    let (status, _proposal) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions"),
        serde_json::json!({
            "proposal": "test",
            "candidate_core": {"mission": "new"},
            "reason": "test",
            "source_event_ids": event_ids,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // List promotions and verify the new one appears as pending with empty approvers
    let (status, list) = json_request(
        &app,
        "GET",
        &format!("/api/v1/agents/{agent}/promotions"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let proposals = list.as_array().unwrap();
    assert!(!proposals.is_empty());
    let p = &proposals[0];
    assert_eq!(p["status"], "pending");
    assert!(p["approvers"].as_array().unwrap().is_empty());
    assert!(p["expired_at"].is_null());
}

// F5-INT-12: Conflict analysis for proposal with no relevant events
#[tokio::test]
async fn f5_conflict_analysis_no_relevant_events() {
    let app = build_test_app().await;
    let agent = "f5-no-conflict";

    // Create experience events about a DIFFERENT topic
    let (_, ev1) = json_request(
        &app, "POST", &format!("/api/v1/agents/{agent}/experience"),
        serde_json::json!({"category":"greeting","signal":"user says hello warmly","confidence":0.9}),
    ).await;
    let (_, ev2) = json_request(
        &app, "POST", &format!("/api/v1/agents/{agent}/experience"),
        serde_json::json!({"category":"greeting","signal":"user prefers casual greetings","confidence":0.8}),
    ).await;
    let (_, ev3) = json_request(
        &app, "POST", &format!("/api/v1/agents/{agent}/experience"),
        serde_json::json!({"category":"greeting","signal":"warm welcomes appreciated","confidence":0.85}),
    ).await;

    let event_ids: Vec<String> = vec![
        ev1["id"].as_str().unwrap().into(),
        ev2["id"].as_str().unwrap().into(),
        ev3["id"].as_str().unwrap().into(),
    ];

    // Create proposal about billing (unrelated to greeting events)
    let (status, proposal) = json_request(
        &app,
        "POST",
        &format!("/api/v1/agents/{agent}/promotions"),
        serde_json::json!({
            "proposal": "add billing expertise",
            "candidate_core": {"capabilities": ["billing", "refunds"]},
            "reason": "expanding capabilities",
            "source_event_ids": event_ids,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = proposal["id"].as_str().unwrap();

    let (status, analysis) = json_request(
        &app,
        "GET",
        &format!("/api/v1/agents/{agent}/promotions/{pid}/conflicts"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(analysis["conflict_score"], 0.0);
    assert_eq!(analysis["recommendation"], "proceed");
}

// F5-INT-13: Webhook event types include promotion types
#[tokio::test]
async fn f5_webhook_event_types_include_promotion_types() {
    // Verify the new webhook event types serialize correctly
    let types = [
        (
            "promotion_proposed",
            serde_json::json!("promotion_proposed"),
        ),
        (
            "promotion_approved",
            serde_json::json!("promotion_approved"),
        ),
        (
            "promotion_rejected",
            serde_json::json!("promotion_rejected"),
        ),
        ("promotion_expired", serde_json::json!("promotion_expired")),
        (
            "promotion_conflict_detected",
            serde_json::json!("promotion_conflict_detected"),
        ),
    ];

    use mnemo_server::state::MemoryWebhookEventType;
    let variants = [
        MemoryWebhookEventType::PromotionProposed,
        MemoryWebhookEventType::PromotionApproved,
        MemoryWebhookEventType::PromotionRejected,
        MemoryWebhookEventType::PromotionExpired,
        MemoryWebhookEventType::PromotionConflictDetected,
    ];

    for (expected_str, variant) in types.iter().zip(variants.iter()) {
        let serialized = serde_json::to_value(variant).unwrap();
        assert_eq!(
            &serialized, &expected_str.1,
            "webhook event type mismatch for {}",
            expected_str.0
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// Feature 6: Multi-Agent Shared Memory with ACLs
// ═══════════════════════════════════════════════════════════════════

/// Helper: create a user and return user_id
async fn create_test_user(app: &axum::Router, external_id: &str) -> String {
    let (status, body) = json_request(
        app,
        "POST",
        "/api/v1/users",
        serde_json::json!({"external_id": external_id, "name": external_id}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create user failed: {body:?}");
    body["id"].as_str().unwrap().to_string()
}

// F6-INT-01: Create a memory region
#[tokio::test]
async fn f6_create_memory_region() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-01").await;

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "shared_customer_context",
            "owner_agent_id": "support-bot",
            "user_id": user_id,
            "classification_ceiling": "confidential",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create region failed: {body:?}"
    );
    assert_eq!(body["name"], "shared_customer_context");
    assert_eq!(body["owner_agent_id"], "support-bot");
    assert_eq!(body["classification_ceiling"], "confidential");
    assert!(body["id"].is_string());
}

// F6-INT-02: Get region by ID
#[tokio::test]
async fn f6_get_region_by_id() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-02").await;

    let (status, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "get_test_region",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let region_id = created["id"].as_str().unwrap();

    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions/{region_id}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get region failed: {body:?}");
    assert_eq!(body["name"], "get_test_region");
    assert_eq!(body["id"], region_id);
}

// F6-INT-03: Get nonexistent region returns 404
#[tokio::test]
async fn f6_get_nonexistent_region_404() {
    let app = build_test_app().await;
    let fake_id = uuid::Uuid::from_u128(999);
    let (status, body) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions/{fake_id}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "should be 404: {body:?}");
}

// F6-INT-04: Update region name and classification
#[tokio::test]
async fn f6_update_region() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-04").await;

    let (_, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "original_name",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = created["id"].as_str().unwrap();

    let (status, updated) = json_request(
        &app,
        "PUT",
        &format!("/api/v1/regions/{region_id}"),
        serde_json::json!({
            "name": "updated_name",
            "classification_ceiling": "restricted",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {updated:?}");
    assert_eq!(updated["name"], "updated_name");
    assert_eq!(updated["classification_ceiling"], "restricted");
}

// F6-INT-05: Delete region
#[tokio::test]
async fn f6_delete_region() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-05").await;

    let (_, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "to_delete",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = created["id"].as_str().unwrap();

    let (status, _) = json_request(
        &app,
        "DELETE",
        &format!("/api/v1/regions/{region_id}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify it's gone
    let (status, _) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions/{region_id}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// F6-INT-06: Grant agent access to a region
#[tokio::test]
async fn f6_grant_region_access() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-06").await;

    let (_, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "shared_region",
            "owner_agent_id": "support-bot",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = created["id"].as_str().unwrap();

    let (status, acl) = json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({
            "agent_id": "sales-bot",
            "permission": "read",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "grant failed: {acl:?}");
    assert_eq!(acl["agent_id"], "sales-bot");
    assert_eq!(acl["permission"], "read");
    assert_eq!(acl["region_id"], region_id);
}

// F6-INT-07: List ACLs for a region
#[tokio::test]
async fn f6_list_region_acls() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-07").await;

    let (_, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "acl_list_region",
            "owner_agent_id": "support-bot",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = created["id"].as_str().unwrap();

    // Grant access to two agents
    json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({"agent_id": "sales-bot", "permission": "read"}),
    )
    .await;
    json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({"agent_id": "billing-bot", "permission": "write"}),
    )
    .await;

    let (status, acls) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let acl_list = acls.as_array().unwrap();
    assert_eq!(acl_list.len(), 2);
}

// F6-INT-08: Revoke region access
#[tokio::test]
async fn f6_revoke_region_access() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-08").await;

    let (_, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "revoke_test",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = created["id"].as_str().unwrap();

    // Grant then revoke
    json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({"agent_id": "sales-bot", "permission": "read"}),
    )
    .await;

    let (status, _) = json_request(
        &app,
        "DELETE",
        &format!("/api/v1/regions/{region_id}/acl/sales-bot"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify ACL list is now empty
    let (status, acls) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(acls.as_array().unwrap().is_empty());
}

// F6-INT-09: Cannot grant access to region owner
#[tokio::test]
async fn f6_cannot_grant_access_to_owner() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-09").await;

    let (_, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "owner_test",
            "owner_agent_id": "bot-owner",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = created["id"].as_str().unwrap();

    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({"agent_id": "bot-owner", "permission": "manage"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "should reject: {body:?}");
}

// F6-INT-10: Create region with entity and edge filters
#[tokio::test]
async fn f6_create_region_with_filters() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-10").await;

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "filtered_region",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
            "entity_filter": {
                "entity_types": ["person", "organization"],
                "name_patterns": ["acme"],
            },
            "edge_filter": {
                "labels": ["works_at", "reports_to"],
                "min_confidence": 0.7,
            },
            "classification_ceiling": "confidential",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create filtered failed: {body:?}"
    );
    assert_eq!(
        body["entity_filter"]["entity_types"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(body["edge_filter"]["labels"].as_array().unwrap().len(), 2);
    assert_eq!(body["edge_filter"]["min_confidence"], 0.7);
}

// F6-INT-11: List regions filtered by agent_id
#[tokio::test]
async fn f6_list_regions_by_agent() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-11").await;

    // Create two regions owned by different agents
    json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "bot_a_region",
            "owner_agent_id": "f6-list-bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "bot_b_region",
            "owner_agent_id": "f6-list-bot-b",
            "user_id": user_id,
        }),
    )
    .await;

    // List for bot-a should return 1
    let (status, body) = json_request(
        &app,
        "GET",
        "/api/v1/regions?agent_id=f6-list-bot-a",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let regions = body.as_array().unwrap();
    assert!(regions
        .iter()
        .all(|r| r["owner_agent_id"] == "f6-list-bot-a"));
}

// F6-INT-12: Delete region cleans up ACLs
#[tokio::test]
async fn f6_delete_region_cleans_up_acls() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-12").await;

    let (_, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "cleanup_test",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = created["id"].as_str().unwrap();

    // Grant access
    json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({"agent_id": "bot-b", "permission": "read"}),
    )
    .await;

    // Delete region
    let (status, _) = json_request(
        &app,
        "DELETE",
        &format!("/api/v1/regions/{region_id}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify region and ACLs are gone
    let (status, _) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions/{region_id}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// F6-INT-13: Grant access with expiry
#[tokio::test]
async fn f6_grant_access_with_expiry() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-13").await;

    let (_, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "expiry_test",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = created["id"].as_str().unwrap();

    let (status, acl) = json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({
            "agent_id": "bot-b",
            "permission": "read",
            "expires_at": "2099-12-31T23:59:59Z",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(acl["expires_at"].is_string());
}

// F6-INT-14: Validate region name — empty rejected
#[tokio::test]
async fn f6_create_region_empty_name_rejected() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-14").await;

    let (status, body) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "empty name should fail: {body:?}"
    );
}

// F6-INT-15: Grant access with empty agent_id rejected
#[tokio::test]
async fn f6_grant_empty_agent_id_rejected() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-15").await;

    let (_, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "empty_agent_test",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = created["id"].as_str().unwrap();

    let (status, body) = json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({"agent_id": "", "permission": "read"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "empty agent_id should fail: {body:?}"
    );
}

// F6-INT-16: RBAC — delete region requires Admin
#[tokio::test]
async fn f6_delete_region_requires_admin() {
    let (app, _store, _admin_key) =
        build_authed_test_app_with_store(vec!["f6-admin-key".to_string()]).await;

    // Create a read key
    let (_, key_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/keys",
        "authorization",
        "Bearer f6-admin-key",
        serde_json::json!({"name": "f6-write-key", "role": "write"}),
    )
    .await;
    let write_key = key_body["raw_key"].as_str().unwrap().to_string();

    // Create user and region
    let (_, user_body) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/users",
        "authorization",
        "Bearer f6-admin-key",
        serde_json::json!({"external_id": "f6-rbac-user", "name": "f6-rbac-user"}),
    )
    .await;
    let user_id = user_body["id"].as_str().unwrap();

    let (status, created) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/regions",
        "authorization",
        "Bearer f6-admin-key",
        serde_json::json!({
            "name": "rbac_delete_test",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let region_id = created["id"].as_str().unwrap();

    // Write key should be forbidden from deleting
    let (status, body) = json_request_with_header(
        &app,
        "DELETE",
        &format!("/api/v1/regions/{region_id}"),
        "authorization",
        &format!("Bearer {write_key}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "write key should be forbidden: {body:?}"
    );

    // Admin key should succeed
    let (status, _) = json_request_with_header(
        &app,
        "DELETE",
        &format!("/api/v1/regions/{region_id}"),
        "authorization",
        "Bearer f6-admin-key",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

// F6-INT-17: Upsert ACL — granting again replaces permission
#[tokio::test]
async fn f6_upsert_acl_replaces_permission() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "f6-user-17").await;

    let (_, created) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "upsert_test",
            "owner_agent_id": "bot-a",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = created["id"].as_str().unwrap();

    // Grant Read
    json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({"agent_id": "bot-b", "permission": "read"}),
    )
    .await;

    // Upgrade to Write
    let (status, acl) = json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({"agent_id": "bot-b", "permission": "write"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(acl["permission"], "write");

    // Verify only 1 ACL entry (not duplicated)
    let (_, acls) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(acls.as_array().unwrap().len(), 1);
    assert_eq!(acls[0]["permission"], "write");
}

// ═══════════════════════════════════════════════════════════════════
// Feature 6 — Red-team / Adversarial Tests
// ═══════════════════════════════════════════════════════════════════

/// Helper: create a user using an authed app
async fn create_test_user_authed(app: &axum::Router, admin_key: &str, external_id: &str) -> String {
    let (status, body) = json_request_with_header(
        app,
        "POST",
        "/api/v1/users",
        "authorization",
        &format!("Bearer {admin_key}"),
        serde_json::json!({"external_id": external_id, "name": external_id}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create user failed: {body:?}");
    body["id"].as_str().unwrap().to_string()
}

/// Helper: create a write key with a specific name (used as owner identity)
async fn create_named_write_key(app: &axum::Router, admin_key: &str, name: &str) -> String {
    let (status, body) = json_request_with_header(
        app,
        "POST",
        "/api/v1/keys",
        "authorization",
        &format!("Bearer {admin_key}"),
        serde_json::json!({"name": name, "role": "write"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create key failed: {body:?}");
    body["raw_key"].as_str().unwrap().to_string()
}

/// Helper: create a read key
async fn create_read_key(app: &axum::Router, admin_key: &str, name: &str) -> String {
    let (status, body) = json_request_with_header(
        app,
        "POST",
        "/api/v1/keys",
        "authorization",
        &format!("Bearer {admin_key}"),
        serde_json::json!({"name": name, "role": "read"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create key failed: {body:?}");
    body["raw_key"].as_str().unwrap().to_string()
}

// RT-01: Read key cannot create regions
#[tokio::test]
async fn f6_rt_read_key_cannot_create_region() {
    let (app, _store, admin_key) =
        build_authed_test_app_with_store(vec!["rt-admin-01".to_string()]).await;
    let read_key = create_read_key(&app, &admin_key, "read-agent").await;
    let user_id = create_test_user_authed(&app, &admin_key, "rt-user-01").await;

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/regions",
        "authorization",
        &format!("Bearer {read_key}"),
        serde_json::json!({
            "name": "stolen_region",
            "owner_agent_id": "read-agent",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// RT-02: Read key cannot grant ACLs
#[tokio::test]
async fn f6_rt_read_key_cannot_grant_acl() {
    let (app, _store, admin_key) =
        build_authed_test_app_with_store(vec!["rt-admin-02".to_string()]).await;
    let read_key = create_read_key(&app, &admin_key, "read-agent").await;
    let user_id = create_test_user_authed(&app, &admin_key, "rt-user-02").await;

    // Admin creates a region
    let (_, region) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/regions",
        "authorization",
        &format!("Bearer {admin_key}"),
        serde_json::json!({
            "name": "protected_region",
            "owner_agent_id": "rt-admin-02",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = region["id"].as_str().unwrap();

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        "authorization",
        &format!("Bearer {read_key}"),
        serde_json::json!({"agent_id": "evil-bot", "permission": "manage"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// RT-03: Non-owner write key cannot grant ACLs on someone else's region
#[tokio::test]
async fn f6_rt_nonowner_cannot_grant_acl() {
    let (app, _store, admin_key) =
        build_authed_test_app_with_store(vec!["rt-admin-03".to_string()]).await;
    let owner_key = create_named_write_key(&app, &admin_key, "owner-bot").await;
    let attacker_key = create_named_write_key(&app, &admin_key, "attacker-bot").await;
    let user_id = create_test_user_authed(&app, &admin_key, "rt-user-03").await;

    // Owner creates a region
    let (status, region) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/regions",
        "authorization",
        &format!("Bearer {owner_key}"),
        serde_json::json!({
            "name": "owners_region",
            "owner_agent_id": "owner-bot",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let region_id = region["id"].as_str().unwrap();

    // Attacker tries to grant themselves access
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        "authorization",
        &format!("Bearer {attacker_key}"),
        serde_json::json!({"agent_id": "attacker-bot", "permission": "manage"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "non-owner should not be able to grant ACLs"
    );
}

// RT-04: Non-owner write key cannot revoke ACLs
#[tokio::test]
async fn f6_rt_nonowner_cannot_revoke_acl() {
    let (app, _store, admin_key) =
        build_authed_test_app_with_store(vec!["rt-admin-04".to_string()]).await;
    let owner_key = create_named_write_key(&app, &admin_key, "owner-bot-04").await;
    let attacker_key = create_named_write_key(&app, &admin_key, "attacker-bot-04").await;
    let user_id = create_test_user_authed(&app, &admin_key, "rt-user-04").await;

    // Owner creates region and grants access to a legit agent
    let (_, region) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/regions",
        "authorization",
        &format!("Bearer {owner_key}"),
        serde_json::json!({
            "name": "owners_region_04",
            "owner_agent_id": "owner-bot-04",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = region["id"].as_str().unwrap();

    let (status, _) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        "authorization",
        &format!("Bearer {owner_key}"),
        serde_json::json!({"agent_id": "legit-agent", "permission": "read"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Attacker tries to revoke legit agent's access
    let (status, _) = json_request_with_header(
        &app,
        "DELETE",
        &format!("/api/v1/regions/{region_id}/acl/legit-agent"),
        "authorization",
        &format!("Bearer {attacker_key}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "non-owner should not be able to revoke ACLs"
    );
}

// RT-05: Non-owner write key cannot update a region
#[tokio::test]
async fn f6_rt_nonowner_cannot_update_region() {
    let (app, _store, admin_key) =
        build_authed_test_app_with_store(vec!["rt-admin-05".to_string()]).await;
    let owner_key = create_named_write_key(&app, &admin_key, "owner-bot-05").await;
    let attacker_key = create_named_write_key(&app, &admin_key, "attacker-bot-05").await;
    let user_id = create_test_user_authed(&app, &admin_key, "rt-user-05").await;

    let (_, region) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/regions",
        "authorization",
        &format!("Bearer {owner_key}"),
        serde_json::json!({
            "name": "immutable_region",
            "owner_agent_id": "owner-bot-05",
            "user_id": user_id,
            "classification_ceiling": "confidential",
        }),
    )
    .await;
    let region_id = region["id"].as_str().unwrap();

    // Attacker tries to widen classification ceiling
    let (status, _) = json_request_with_header(
        &app,
        "PUT",
        &format!("/api/v1/regions/{region_id}"),
        "authorization",
        &format!("Bearer {attacker_key}"),
        serde_json::json!({"classification_ceiling": "restricted"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "non-owner should not be able to update region"
    );
}

// RT-06: Owner CAN grant, update, and revoke on their own region
#[tokio::test]
async fn f6_rt_owner_can_manage_own_region() {
    let (app, _store, admin_key) =
        build_authed_test_app_with_store(vec!["rt-admin-06".to_string()]).await;
    let owner_key = create_named_write_key(&app, &admin_key, "owner-bot-06").await;
    let user_id = create_test_user_authed(&app, &admin_key, "rt-user-06").await;

    let (status, region) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/regions",
        "authorization",
        &format!("Bearer {owner_key}"),
        serde_json::json!({
            "name": "managed_region",
            "owner_agent_id": "owner-bot-06",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let region_id = region["id"].as_str().unwrap();

    // Owner grants access
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        "authorization",
        &format!("Bearer {owner_key}"),
        serde_json::json!({"agent_id": "helper-bot", "permission": "write"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Owner updates region
    let (status, _) = json_request_with_header(
        &app,
        "PUT",
        &format!("/api/v1/regions/{region_id}"),
        "authorization",
        &format!("Bearer {owner_key}"),
        serde_json::json!({"name": "renamed_region"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Owner revokes access
    let (status, _) = json_request_with_header(
        &app,
        "DELETE",
        &format!("/api/v1/regions/{region_id}/acl/helper-bot"),
        "authorization",
        &format!("Bearer {owner_key}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

// RT-07: Admin can grant/revoke even on non-owned regions
#[tokio::test]
async fn f6_rt_admin_can_manage_any_region() {
    let (app, _store, admin_key) =
        build_authed_test_app_with_store(vec!["rt-admin-07".to_string()]).await;
    let owner_key = create_named_write_key(&app, &admin_key, "owner-bot-07").await;
    let user_id = create_test_user_authed(&app, &admin_key, "rt-user-07").await;

    let (_, region) = json_request_with_header(
        &app,
        "POST",
        "/api/v1/regions",
        "authorization",
        &format!("Bearer {owner_key}"),
        serde_json::json!({
            "name": "any_region",
            "owner_agent_id": "owner-bot-07",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = region["id"].as_str().unwrap();

    // Admin grants access on someone else's region
    let (status, _) = json_request_with_header(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        "authorization",
        &format!("Bearer {admin_key}"),
        serde_json::json!({"agent_id": "admin-agent", "permission": "manage"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Admin revokes
    let (status, _) = json_request_with_header(
        &app,
        "DELETE",
        &format!("/api/v1/regions/{region_id}/acl/admin-agent"),
        "authorization",
        &format!("Bearer {admin_key}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

// RT-08: Agent ID with colon (key injection) is rejected
#[tokio::test]
async fn f6_rt_agent_id_colon_injection_rejected() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "rt-user-08").await;

    // owner_agent_id with colon
    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "injection_test",
            "owner_agent_id": "evil:agent_regions:victim",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "colon in owner_agent_id should be rejected"
    );
}

// RT-09: Agent ID with path traversal is rejected
#[tokio::test]
async fn f6_rt_agent_id_path_traversal_rejected() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "rt-user-09").await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "traversal_test",
            "owner_agent_id": "../../../etc/passwd",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "path traversal in owner_agent_id should be rejected"
    );
}

// RT-10: Agent ID with null byte is rejected
#[tokio::test]
async fn f6_rt_agent_id_null_byte_rejected() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "rt-user-10").await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "null_test",
            "owner_agent_id": "bot\u{0000}hidden",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "null byte in owner_agent_id should be rejected"
    );
}

// RT-11: Grant ACL with injected agent_id is rejected
#[tokio::test]
async fn f6_rt_grant_acl_agent_id_injection_rejected() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "rt-user-11").await;

    let (_, region) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "acl_inject_test",
            "owner_agent_id": "safe-bot",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = region["id"].as_str().unwrap();

    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({"agent_id": "evil:key:injection", "permission": "manage"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "colon in grant agent_id should be rejected"
    );
}

// RT-12: Create region with nonexistent user_id is rejected
#[tokio::test]
async fn f6_rt_create_region_nonexistent_user_rejected() {
    let app = build_test_app().await;
    let fake_user_id = Uuid::from_u128(999_999);

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "orphan_region",
            "owner_agent_id": "some-bot",
            "user_id": fake_user_id,
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "creating a region for a nonexistent user should fail"
    );
}

// RT-13: Expired ACLs are not returned by list_region_acls
#[tokio::test]
async fn f6_rt_expired_acls_filtered_from_list() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "rt-user-13").await;

    let (_, region) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "expiry_test",
            "owner_agent_id": "bot-owner",
            "user_id": user_id,
        }),
    )
    .await;
    let region_id = region["id"].as_str().unwrap();

    // Grant with already-expired timestamp
    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({
            "agent_id": "temp-bot",
            "permission": "read",
            "expires_at": "2020-01-01T00:00:00Z",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // List should NOT include the expired ACL
    let (status, acls) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        acls.as_array().unwrap().len(),
        0,
        "expired ACLs should be filtered from listing"
    );
}

// RT-14: Expired ACLs don't appear in list_regions(agent_id) either
#[tokio::test]
async fn f6_rt_expired_acl_agent_not_in_region_listing() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "rt-user-14").await;

    let (_, region) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "listing_expiry_test",
            "owner_agent_id": "bot-owner-14",
            "user_id": user_id,
        }),
    )
    .await;
    let _region_id = region["id"].as_str().unwrap();

    // Grant with already-expired timestamp
    let region_id = region["id"].as_str().unwrap();
    let (status, _) = json_request(
        &app,
        "POST",
        &format!("/api/v1/regions/{region_id}/acl"),
        serde_json::json!({
            "agent_id": "expired-bot",
            "permission": "write",
            "expires_at": "2020-01-01T00:00:00Z",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // expired-bot should NOT see this region in their listing
    let (status, regions) = json_request(
        &app,
        "GET",
        "/api/v1/regions?agent_id=expired-bot",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        regions.as_array().unwrap().len(),
        0,
        "expired ACL should not cause region to appear in agent listing"
    );
}

// RT-15: Empty owner_agent_id is rejected
#[tokio::test]
async fn f6_rt_empty_owner_agent_id_rejected() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "rt-user-15").await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "empty_owner_test",
            "owner_agent_id": "",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "empty owner_agent_id should be rejected"
    );
}

// RT-16: Unicode in agent_id is rejected (prevents key encoding issues)
#[tokio::test]
async fn f6_rt_unicode_agent_id_rejected() {
    let app = build_test_app().await;
    let user_id = create_test_user(&app, "rt-user-16").await;

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "unicode_test",
            "owner_agent_id": "böt-ünïcödé",
            "user_id": user_id,
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unicode in owner_agent_id should be rejected"
    );
}

// RT-17: list_regions with user_id filter scopes correctly
#[tokio::test]
async fn f6_rt_list_regions_user_scoped() {
    let app = build_test_app().await;
    let user_a = create_test_user(&app, "rt-user-scope-a").await;
    let user_b = create_test_user(&app, "rt-user-scope-b").await;

    // Create a region for each user
    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "region_user_a",
            "owner_agent_id": "bot-a",
            "user_id": user_a,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "region_user_b",
            "owner_agent_id": "bot-b",
            "user_id": user_b,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // List regions for user_a only
    let (status, regions) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions?user_id={user_a}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let arr = regions.as_array().unwrap();
    assert_eq!(arr.len(), 1, "should only see user_a's region");
    assert_eq!(arr[0]["name"], "region_user_a");

    // List regions for user_b only
    let (status, regions) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions?user_id={user_b}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let arr = regions.as_array().unwrap();
    assert_eq!(arr.len(), 1, "should only see user_b's region");
    assert_eq!(arr[0]["name"], "region_user_b");
}

// RT-18: list_regions with user_id + agent_id combined filtering
#[tokio::test]
async fn f6_rt_list_regions_user_and_agent_scoped() {
    let app = build_test_app().await;
    let user_a = create_test_user(&app, "rt-user-combo-a").await;
    let user_b = create_test_user(&app, "rt-user-combo-b").await;

    // Same agent owns regions for different users
    json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "combo_region_a",
            "owner_agent_id": "shared-bot",
            "user_id": user_a,
        }),
    )
    .await;

    json_request(
        &app,
        "POST",
        "/api/v1/regions",
        serde_json::json!({
            "name": "combo_region_b",
            "owner_agent_id": "shared-bot",
            "user_id": user_b,
        }),
    )
    .await;

    // Filter by agent + user_a → should only see user_a's region
    let (status, regions) = json_request(
        &app,
        "GET",
        &format!("/api/v1/regions?agent_id=shared-bot&user_id={user_a}"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let arr = regions.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "combo_region_a");
}

// RT-19: Lazy cleanup removes expired ACL entries from indices
#[tokio::test]
async fn f6_rt_lazy_cleanup_expired_acls() {
    let (_, store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    // Create user and region directly via store
    let user_id = Uuid::now_v7();
    store
        .create_user(mnemo_core::models::user::CreateUserRequest {
            id: Some(user_id),
            external_id: Some("cleanup-user".into()),
            name: "cleanup-user".into(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let region_id = Uuid::now_v7();
    let region = mnemo_core::models::region::MemoryRegion {
        id: region_id,
        name: "cleanup_test".into(),
        owner_agent_id: "owner-bot".into(),
        user_id,
        entity_filter: None,
        edge_filter: None,
        classification_ceiling: mnemo_core::models::classification::Classification::Internal,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    store.create_region(&region).await.unwrap();

    // Grant an ACL that is already expired
    let expired_acl = mnemo_core::models::region::MemoryRegionAcl {
        region_id,
        agent_id: "expired-agent".into(),
        permission: mnemo_core::models::region::RegionPermission::Read,
        granted_by: "owner-bot".into(),
        granted_at: chrono::Utc::now() - chrono::Duration::hours(48),
        expires_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
    };
    store.grant_region_access(&expired_acl).await.unwrap();

    // Raw ACL list should still contain it (list_region_acls doesn't filter)
    let raw_acls = store.list_region_acls(region_id).await.unwrap();
    assert_eq!(raw_acls.len(), 1, "raw list should have the expired ACL");

    // list_agent_accessible_regions triggers lazy cleanup
    let accessible = store
        .list_agent_accessible_regions("expired-agent")
        .await
        .unwrap();
    assert_eq!(accessible.len(), 0, "expired ACL should not grant access");

    // After lazy cleanup, the raw ACL list should be empty
    // Give Redis a moment to process the pipeline
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let raw_acls_after = store.list_region_acls(region_id).await.unwrap();
    assert_eq!(
        raw_acls_after.len(),
        0,
        "lazy cleanup should have removed the expired ACL entry"
    );
}

// RT-20: delete_region atomically removes all indices (verify via raw store)
#[tokio::test]
async fn f6_rt_delete_region_cleans_all_indices() {
    let (_, store) = build_test_harness_with_prefilter(MetadataPrefilterConfig {
        enabled: false,
        scan_limit: 400,
        relax_if_empty: false,
    })
    .await;

    let user_id = Uuid::now_v7();
    store
        .create_user(mnemo_core::models::user::CreateUserRequest {
            id: Some(user_id),
            external_id: Some("idx-user".into()),
            name: "idx-user".into(),
            email: None,
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();

    let region_id = Uuid::now_v7();
    let region = mnemo_core::models::region::MemoryRegion {
        id: region_id,
        name: "idx_test".into(),
        owner_agent_id: "idx-owner".into(),
        user_id,
        entity_filter: None,
        edge_filter: None,
        classification_ceiling: mnemo_core::models::classification::Classification::Internal,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    store.create_region(&region).await.unwrap();

    // Grant ACL to another agent
    let acl = mnemo_core::models::region::MemoryRegionAcl {
        region_id,
        agent_id: "guest-agent".into(),
        permission: mnemo_core::models::region::RegionPermission::Write,
        granted_by: "idx-owner".into(),
        granted_at: chrono::Utc::now(),
        expires_at: None,
    };
    store.grant_region_access(&acl).await.unwrap();

    // Verify region appears in listings
    let owner_regions = store
        .list_agent_accessible_regions("idx-owner")
        .await
        .unwrap();
    assert_eq!(owner_regions.len(), 1);
    let guest_regions = store
        .list_agent_accessible_regions("guest-agent")
        .await
        .unwrap();
    assert_eq!(guest_regions.len(), 1);
    let user_regions = store.list_regions(Some(user_id), None).await.unwrap();
    assert_eq!(user_regions.len(), 1);

    // Delete the region
    store.delete_region(region_id).await.unwrap();

    // Verify complete cleanup
    assert!(
        store.get_region(region_id).await.unwrap().is_none(),
        "region document should be gone"
    );
    let owner_regions = store
        .list_agent_accessible_regions("idx-owner")
        .await
        .unwrap();
    assert_eq!(owner_regions.len(), 0, "owner index should be clean");
    let guest_regions = store
        .list_agent_accessible_regions("guest-agent")
        .await
        .unwrap();
    assert_eq!(guest_regions.len(), 0, "guest agent index should be clean");
    let user_regions = store.list_regions(Some(user_id), None).await.unwrap();
    assert_eq!(user_regions.len(), 0, "user index should be clean");
    let acls = store.list_region_acls(region_id).await.unwrap();
    assert_eq!(acls.len(), 0, "ACL entries should be gone");
}
